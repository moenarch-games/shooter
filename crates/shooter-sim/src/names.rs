//! Player names: what a person picked, as the host keeps and shows it. No accounts; the client
//! remembers its own name locally and the host sanitizes whatever arrives ([`sanitize`]).

/// The longest name, in characters.
pub const MAX_NAME_CHARS: usize = 16;

/// `raw` as a name: ASCII letters, digits, `-`, `_`, `.` and single spaces, trimmed and cut to
/// [`MAX_NAME_CHARS`]; anything else is dropped. `None` if nothing is left.
#[must_use]
pub fn sanitize(raw: &str) -> Option<String> {
    let mut name = String::new();
    for character in raw.chars() {
        if character.is_whitespace() {
            if !name.is_empty() && !name.ends_with(' ') {
                name.push(' ');
            }
        } else if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
            name.push(character);
        }
        if name.len() == MAX_NAME_CHARS {
            break;
        }
    }
    let name = name.trim_end();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_short_plain_name() {
        assert_eq!(sanitize("Ripley").as_deref(), Some("Ripley"));
        assert_eq!(sanitize("  Dr.  Who_2 ").as_deref(), Some("Dr. Who_2"));
    }

    #[test]
    fn drops_other_characters_and_cuts_the_length() {
        assert_eq!(sanitize("<b>Zoë</b>\n").as_deref(), Some("bZob"));
        assert_eq!(
            sanitize("abcdefghijklmnopqrstuvwxyz").as_deref(),
            Some("abcdefghijklmnop")
        );
        assert_eq!(
            sanitize("abcdefghijklmno  x").as_deref(),
            Some("abcdefghijklmno")
        );
        assert_eq!(sanitize(" \t\u{200b}☠ "), None);
        assert_eq!(sanitize(""), None);
    }
}
