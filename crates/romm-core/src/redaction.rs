pub fn redact(input: &str) -> String {
    if input.to_ascii_lowercase().contains("authorization:") {
        return "[REDACTED_AUTHORIZATION]".to_owned();
    }

    let characters: Vec<char> = input.chars().collect();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < characters.len() {
        if starts_client_token(&characters, index) {
            output.push_str("[REDACTED_TOKEN]");
            index += 68;
        } else if starts_pairing_code(&characters, index) {
            output.push_str("[REDACTED_PAIRING_CODE]");
            index += 9;
        } else {
            output.push(characters[index]);
            index += 1;
        }
    }
    redact_url_queries(&output)
}

fn starts_client_token(value: &[char], index: usize) -> bool {
    value.get(index..index + 68).is_some_and(|candidate| {
        candidate[0..4] == ['r', 'm', 'm', '_']
            && candidate[4..]
                .iter()
                .all(|character| character.is_ascii_hexdigit())
    })
}

fn starts_pairing_code(value: &[char], index: usize) -> bool {
    value.get(index..index + 9).is_some_and(|candidate| {
        candidate[0..4]
            .iter()
            .chain(candidate[5..9].iter())
            .all(|character| character.is_ascii_alphanumeric())
            && candidate[4] == '-'
            && (index == 0 || !value[index - 1].is_ascii_alphanumeric())
            && value
                .get(index + 9)
                .is_none_or(|character| !character.is_ascii_alphanumeric())
    })
}

fn redact_url_queries(input: &str) -> String {
    input
        .split_whitespace()
        .map(|part| {
            if (part.starts_with("http://") || part.starts_with("https://"))
                && let Ok(mut url) = url::Url::parse(part)
            {
                if !url.username().is_empty() || url.password().is_some() {
                    let _ = url.set_username("");
                    let _ = url.set_password(None);
                }
                if url.query().is_some() {
                    url.set_query(Some("REDACTED"));
                }
                return url.to_string();
            }
            part.to_owned()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credentials_codes_and_sensitive_urls() {
        let token = format!("rmm_{}", "a".repeat(64));
        assert_eq!(redact(&token), "[REDACTED_TOKEN]");
        assert_eq!(
            redact("pair JM38-MHSA now"),
            "pair [REDACTED_PAIRING_CODE] now"
        );
        assert_eq!(
            redact("GET https://user:pass@host.test/api?token=secret"),
            "GET https://host.test/api?REDACTED"
        );
        assert_eq!(
            redact("Authorization: Bearer anything"),
            "[REDACTED_AUTHORIZATION]"
        );
    }
}
