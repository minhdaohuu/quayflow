//! Console output. On a terminal Rust writes UTF-8 through the console API, so `·` and `▶` show
//! correctly. When the output is piped, Windows PowerShell decodes it with the console's legacy
//! code page and those symbols turn into `┬╖`; there the CLI's own symbols become ASCII. Text
//! the person typed (account labels, group names) is never changed.

use std::borrow::Cow;
use std::io::IsTerminal;
use std::sync::OnceLock;

fn piped() -> bool {
    static PIPED: OnceLock<bool> = OnceLock::new();
    *PIPED.get_or_init(|| !std::io::stdout().is_terminal())
}

/// The CLI's own symbols, made ASCII when stdout is not a terminal.
pub fn fit(s: &str) -> Cow<'_, str> {
    if piped() {
        ascii(s)
    } else {
        Cow::Borrowed(s)
    }
}

fn ascii(s: &str) -> Cow<'_, str> {
    if s.is_ascii() || !s.contains(['·', '—', '→', '▶']) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(s.replace(['·', '—'], "-").replace('→', "->").replace('▶', ">"))
}

#[cfg(test)]
mod tests {
    use super::ascii;

    #[test]
    fn only_the_cli_symbols_change() {
        assert_eq!(ascii("qf-e2e · no login"), "qf-e2e - no login");
        assert_eq!(ascii("▶ work — 8% → ok"), "> work - 8% -> ok");
        assert_eq!(ascii("Nguyễn · công ty"), "Nguyễn - công ty");
        assert_eq!(ascii("plain"), "plain");
    }
}
