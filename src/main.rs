mod bot;
mod downloader;
mod helpers;
mod processor;

use std::sync::Arc;

use bot::{TelegramBot, TeloxideBot};
use downloader::Downloader;
use helpers::{status_message::StatusMessage, temp_dir::TempDir};
use once_cell::sync::Lazy;
use processor::song::{DemucsModel, SongProcessor};
use teloxide::{
    payloads::SendMessageSetters,
    prelude::*,
    types::{InputFile, InputMedia, InputMediaAudio},
    utils::command::BotCommands,
};
use tokio::sync::Semaphore;
use tracing::{info, level_filters::LevelFilter, trace, warn};
use tracing_subscriber::{filter::Builder as TracingFilterBuilder, util::SubscriberInitExt};
use url::Url;

static SONG_SEMAPHORE: Lazy<Arc<Semaphore>> = Lazy::new(|| Arc::new(Semaphore::new(1)));

#[tokio::main]
async fn main() {
    match dotenvy::dotenv() {
        Err(e) if e.not_found() => {}
        Ok(_) => {}
        Err(e) => {
            panic!("Failed to load .env file: {}", e);
        }
    }

    init_log();

    info!("Starting command bot...");

    let bot = TelegramBot::instance();

    bot.set_my_commands(Command::bot_commands())
        .send()
        .await
        .expect("Failed to set commands");

    Dispatcher::builder(bot, Update::filter_message().endpoint(answer))
        .build()
        .dispatch()
        .await;
}

#[derive(BotCommands, Debug, Clone)]
#[command(
    rename_rule = "kebab-case",
    description = "These commands are supported:"
)]
enum Command {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "start using the bot.")]
    Start,
}

#[tracing::instrument(skip(bot, msg), fields(chat = %msg.chat.id, msg = %msg.id))]
async fn answer(bot: &TeloxideBot, msg: Message) -> ResponseResult<()> {
    trace!(?msg, "Got message");
    let bot_me = bot.get_me().await?;

    let Some(msg_text) = msg.text() else {
        return Ok(());
    };

    match Command::parse(msg_text, bot_me.username()) {
        Ok(c) => handle_command(bot, msg, c).await,
        Err(_) => handle_message(bot, msg).await,
    }
}

#[tracing::instrument(skip(bot, msg))]
async fn handle_command(bot: &TeloxideBot, msg: Message, cmd: Command) -> ResponseResult<()> {
    trace!("Handling command");

    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }

        Command::Start => {
            bot.send_message(
                msg.chat.id,
                "Just send a link to a song (YouTube, Spotify, Deezer, Tidal...) and the bot will \
                 try and remove the vocals from it!",
            )
            .await?;
        }
    }
    Ok(())
}

#[tracing::instrument(skip_all, fields(chat = ?msg.chat.id, msg = ?msg.id))]
async fn handle_message(bot: &TeloxideBot, msg: Message) -> ResponseResult<()> {
    trace!(?msg, "Handling message");
    let Some(msg_text) = msg.text() else {
        trace!("Message does not contain text");
        return Ok(());
    };

    let parsed_url = match url::Url::parse(msg_text) {
        Ok(u) => u,
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                "Could not parse message URL!\nPlease send a link to the song you want to \
                 karaokify.",
            )
            .reply_to_message_id(msg.id)
            .await?;

            trace!(?e, "Could not parse URL");

            return Ok(());
        }
    };

    tokio::task::spawn(async {
        info!(url = ?parsed_url, "Starting download task");

        let res = process_song(msg.into(), parsed_url).await;

        if let Err(e) = res {
            warn!(?e, "Failed to process song");
        } else {
            info!("Song processed");
        }
    });

    Ok(())
}

#[tracing::instrument(skip_all, fields(chat = ?msg.chat_id(), msg = ?msg.msg_replying_to_id(), url = url.as_str()))]
async fn process_song(mut msg: StatusMessage, url: Url) -> ResponseResult<()> {
    msg.update_message("Waiting in download queue...").await?;

    let _permit = SONG_SEMAPHORE
        .acquire()
        .await
        .expect("Semaphore should not be closed");

    msg.update_message("Downloading song...").await?;

    let temp_dir = TempDir::with_prefix("karaokify-")?;

    let song_file_path = match Downloader::download_song(temp_dir.path(), &url).await {
        Err(e) => {
            msg.update_message(&format!("Download failed.\n\nReason: {e}"))
                .await?;
            return Ok::<(), teloxide::RequestError>(());
        }

        Ok(p) => p,
    };

    trace!(?song_file_path, "Song downloaded");

    msg.update_message("Download finished. Processing song...\n\nThis may take a while.")
        .await?;

    let stem_paths = match SongProcessor::split_into_stems(
        temp_dir.path(),
        &song_file_path,
        DemucsModel::HTDemucs,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            msg.update_message(&format!("Failed to process song.\n\nReason:{e}"))
                .await?;
            return Ok(());
        }
    };

    trace!(?stem_paths, "Stems created");

    msg.update_message("Finished processing song. Uploading files...")
        .await?;

    trace!("Uploading files");
    let media_group = stem_paths
        .into_iter()
        .map(|stem| InputMedia::Audio(InputMediaAudio::new(InputFile::file(stem))))
        .collect::<Vec<_>>();

    TelegramBot::instance()
        .send_media_group(msg.chat_id(), media_group)
        .reply_to_message_id(msg.msg_replying_to_id())
        .send()
        .await?;
    trace!("Files uploaded");

    trace!("Deleting status message");
    msg.delete_message().await?;
    trace!("Status message deleted");

    Ok(())
}

fn init_log() {
    tracing_subscriber::fmt()
        .with_ansi(true)
        .with_env_filter(
            TracingFilterBuilder::default()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .finish()
        .init();
}
