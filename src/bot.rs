use once_cell::sync::OnceCell;
use teloxide::{adaptors::trace, requests::RequesterExt};

pub type TeloxideBot =
    teloxide::adaptors::CacheMe<trace::Trace<teloxide::adaptors::DefaultParseMode<teloxide::Bot>>>;

static TELEGRAM_BOT: OnceCell<TeloxideBot> = OnceCell::new();

pub struct TelegramBot;
impl TelegramBot {
    pub fn instance() -> &'static TeloxideBot {
        TELEGRAM_BOT.get_or_init(|| {
            teloxide::Bot::from_env()
                .parse_mode(teloxide::types::ParseMode::Html)
                .trace(trace::Settings::TRACE_EVERYTHING)
                .cache_me()
        })
    }
}
