mod bot;
mod downloader;
mod helpers;
mod processor;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use bot::{TelegramBot, TeloxideBot};
use downloader::Downloader;
use helpers::{status_message::StatusMessage, temp_dir::TempDir};
use once_cell::sync::Lazy;
use processor::demucs::{DemucsModel, DemucsProcessor};
use teloxide::{
    payloads::SendMessageSetters,
    prelude::*,
    types::{InputFile, InputMedia, InputMediaAudio},
    utils::command::BotCommands,
};
use tokio::sync::Semaphore;
use tracing::{debug, field, info, info_span, level_filters::LevelFilter, trace, warn, Instrument};
use tracing_subscriber::{filter::Builder as TracingFilterBuilder, util::SubscriberInitExt};
use url::Url;

static SONG_SEMAPHORE: Lazy<Arc<Semaphore>> = Lazy::new(|| Arc::new(Semaphore::new(1)));

const MAX_PAYLOAD_SIZE: u64 = {
    let kb = 1000;
    let mb = kb * 1000;

    50 * mb
};

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

    let task_span = {
        let span = info_span!(
        "process_song",
        url = ?parsed_url.as_str(),
        uid = field::Empty,
        user = field::Empty,
        name = field::Empty,
        );

        if let Some(from) = msg.from() {
            if let Some(u) = &from.username {
                span.record("user", field::display(u));
            }
            span.record("uid", field::display(from.id));
            span.record("name", field::display(from.full_name()));
        }

        span
    };

    tokio::task::spawn(
        async {
            info!("Starting download task");

            let res = process_song(msg.into(), parsed_url).await;

            if let Err(e) = res {
                warn!(?e, "Failed to process song");
            } else {
                info!("Song processed");
            }
        }
        .instrument(task_span),
    );

    Ok(())
}

async fn process_song(mut msg: StatusMessage, url: Url) -> ResponseResult<()> {
    msg.update_message("Waiting in queue...").await?;

    let permit = SONG_SEMAPHORE
        .acquire()
        .await
        .expect("Semaphore should not be closed");

    msg.update_message("Downloading song...").await?;

    let temp_dir = TempDir::with_prefix("karaokify-").await?;

    let song_file_path = match Downloader::download_song(temp_dir.path(), &url).await {
        Err(e) => {
            msg.update_message(&format!("Download failed.\n\nReason: {e}"))
                .await?;
            return Ok::<(), teloxide::RequestError>(());
        }

        Ok(p) => p,
    };

    trace!(?song_file_path, "Song downloaded");

    msg.update_message(
        "Download finished. Processing song...\n\nThis will take approximately 2x the song \
         duration.",
    )
    .await?;

    let stem_paths = match DemucsProcessor::split_into_stems(
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

    drop(permit);

    msg.update_message("Finished processing song. Uploading files...")
        .await?;

    let (stem_path_chunks, failed_files) =
        chunk_files_by_size(stem_paths, MAX_PAYLOAD_SIZE / 10 * 8).await;

    trace!("Uploading files");
    for stem_paths in stem_path_chunks {
        trace!(?stem_paths, "Uploading files chunk");
        let media_group = stem_paths
            .into_iter()
            .map(|stem| InputMedia::Audio(InputMediaAudio::new(InputFile::file(stem))))
            .collect::<Vec<_>>();

        TelegramBot::instance()
            .send_media_group(msg.chat_id(), media_group)
            .reply_to_message_id(msg.msg_replying_to_id())
            .allow_sending_without_reply(true)
            .send()
            .await?;
        trace!("Files chunk uploaded");
    }
    trace!("Files uploaded");

    if !failed_files.is_empty() {
        debug!(?failed_files, "Failed to chunk some files to size");
        trace!("Generating failed files message");
        let failed_files_msg = {
            let mut msg = "Failed to upload some files:\n\n".to_string();

            msg += failed_files
                .into_iter()
                .map(|(file, reason)| {
                    format!(
                        " - File: {}\n   Reason: {}\n",
                        file.file_name().unwrap_or_default().to_string_lossy(),
                        reason
                    )
                })
                .reduce(|a, b| a + "\n" + &b)
                .unwrap_or_default()
                .as_str();

            msg
        };
        trace!(msg = ?failed_files_msg, "Failed files message generated");

        trace!("Sending failed files message");
        TelegramBot::instance()
            .send_message(msg.chat_id(), failed_files_msg.trim())
            .reply_to_message_id(msg.msg_replying_to_id())
            .allow_sending_without_reply(true)
            .send()
            .await?;
        trace!("Failed files message sent");
    }

    trace!("Deleting status message");
    msg.delete_message().await?;
    trace!("Status message deleted");

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn chunk_files_by_size(
    files: Vec<PathBuf>,
    max_size: u64,
) -> (Vec<Vec<PathBuf>>, Vec<(PathBuf, String)>) {
    trace!("Calculating file groupings");
    let failed = Arc::new(Mutex::new(Vec::new()));
    let metadatas = {
        let m = files.into_iter().map(|x| {
            let failed = failed.clone();

            async move {
                let meta = match tokio::fs::metadata(&x).await {
                    Ok(meta) => meta,
                    Err(e) => {
                        trace!(?e, "Failed to get metadata for file");
                        if let Ok(mut failed) = failed.lock() {
                            failed.push((x, "failed to get metadata for file".to_string()));
                        }
                        return None;
                    }
                };

                Some((x, meta.len()))
            }
        });

        futures::future::join_all(m)
            .await
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
    };

    let mut res = vec![];
    let mut res_size = 0_u64;
    let mut res_item = vec![];
    for (path, size) in metadatas {
        if size > max_size {
            trace!(?path, ?size, ?max_size, "File is too large");
            if let Ok(mut failed) = failed.lock() {
                failed.push((path, format!("file is too large: {} > {}", size, max_size)));
            }
            continue;
        }

        if size + res_size > max_size {
            res.push(res_item.clone());
            res_size = 0;
            res_item = vec![];
        }

        res_item.push(path);
        res_size += size;
    }
    if !res_item.is_empty() {
        res.push(res_item);
    }
    trace!(?res, "Got file groupings");

    let failed = failed
        .lock()
        .map_or_else(|_| vec![], |failed| failed.iter().cloned().collect());

    trace!(?failed, "Got final failed paths");

    (res, failed)
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
