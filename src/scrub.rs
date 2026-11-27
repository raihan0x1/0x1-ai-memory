#![deny(clippy::string_slice)]

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

struct RedactRule {
    regex: Regex,
    replacement: &'static str,
}

fn rules() -> &'static [RedactRule] {
    static RULES: OnceLock<Vec<RedactRule>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            RedactRule {
                regex: Regex::new(r"(?is)<private>.*?</private>").unwrap(),
                replacement: "[PRIVATE]",
            },
            RedactRule {
                regex: Regex::new(
                    r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
                )
                .unwrap(),
                replacement: "[REDACTED_PRIVATE_KEY]",
            },
            RedactRule {
                regex: Regex::new(
                    r"\b(?:sk-ant-[A-Za-z0-9_-]{16,}|sk-or-v1-[A-Za-z0-9]{16,}|sk-proj-[A-Za-z0-9_-]{16,}|sk-[A-Za-z0-9]{20,})\b",
                )
                .unwrap(),
                replacement: "[REDACTED_API_KEY]",
            },
            RedactRule {
                regex: Regex::new(r"\bAIza[0-9A-Za-z_-]{30,}\b").unwrap(),
                replacement: "[REDACTED_GOOGLE_KEY]",
            },
            RedactRule {
                regex: Regex::new(
                    r"\b(?:ghp_[A-Za-z0-9]{20,}|gho_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|glpat-[A-Za-z0-9_-]{20,}|xox[baprs]-[A-Za-z0-9-]{16,}|AKIA[0-9A-Z]{16})\b",
                )
                .unwrap(),
                replacement: "[REDACTED_TOKEN]",
            },
            RedactRule {
                regex: Regex::new(
                    r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b",
                )
                .unwrap(),
                replacement: "[REDACTED_JWT]",
            },
            RedactRule {
                regex: Regex::new(r"(?i)\b(Bearer\s+)[A-Za-z0-9._~+/=-]{16,}\b").unwrap(),
                replacement: "${1}[REDACTED_BEARER]",
            },
            RedactRule {
                regex: Regex::new(
                    r#"(?i)\b([A-Z0-9_]*(?:API_KEY|SECRET|ACCESS_TOKEN|AUTH_TOKEN|PRIVATE_KEY|PASSWORD|PASSWD|DATABASE_URL|REDIS_URL))\s*([:=])\s*['"]?([^\s'"`]{8,})['"]?"#,
                )
                .unwrap(),
                replacement: "${1}${2}[REDACTED]",
            },
            RedactRule {
                regex: Regex::new(
                    r#"(?i)\b((?:[a-z0-9]+-)*(?:api-key|auth-token|access-token)\s*:\s*)[^\s'"]{8,}"#,
                )
                .unwrap(),
                replacement: "${1}[REDACTED]",
            },
            RedactRule {
                // Password in URL userinfo (`scheme://user:pass@host`); a bare `user@` is kept.
                regex: Regex::new(r"(?i)\b([a-z][a-z0-9+.-]*://[^\s/:@]+:)[^\s/@]+@").unwrap(),
                replacement: "${1}[REDACTED]@",
            },
        ]
    })
}

/// `*_TOKEN=` values of 16+ characters, except purely numeric ones (limits, counters).
fn token_assignment() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)\b([A-Z0-9_]*_TOKEN)(\s*[:=]\s*['"]?)([^\s'"`]{16,})"#).unwrap()
    })
}

/// Deterministically scrubs secrets and `<private>` tags before persistence or LLM dispatch.
pub fn redact_secrets(input: &str) -> String {
    let mut out = input.to_string();
    for rule in rules() {
        if rule.regex.is_match(&out) {
            out = rule.regex.replace_all(&out, rule.replacement).into_owned();
        }
    }
    token_assignment()
        .replace_all(&out, |c: &regex::Captures| {
            if c[3].bytes().all(|b| b.is_ascii_digit()) {
                c[0].to_string()
            } else {
                format!("{}{}[REDACTED]", &c[1], &c[2])
            }
        })
        .into_owned()
}

/// UTF-8 safe head+tail truncation so oversized tool outputs never trigger extra LLM calls.
pub fn truncate_head_tail(input: &str, max_chars: usize) -> String {
    let char_count = input.chars().count();
    if char_count <= max_chars || max_chars < 64 {
        return input.to_string();
    }
    let head_len = (max_chars * 65) / 100;
    let tail_len = max_chars.saturating_sub(head_len + 40);
    let head: String = input.chars().take(head_len).collect();
    let tail: String = input
        .chars()
        .rev()
        .take(tail_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let omitted = char_count.saturating_sub(head_len + tail_len);
    format!("{head}\n... [truncated {omitted} chars] ...\n{tail}")
}

pub fn scrub_and_truncate(input: &str, max_chars: usize) -> String {
    truncate_head_tail(&redact_secrets(input), max_chars)
}

/// Scrubs a JSON value, shrinking the per-field string limit until it serializes within
/// `max_chars`, so a stored payload is never cut mid-JSON.
pub fn scrub_json_bounded(value: &Value, max_chars: usize) -> String {
    let mut out = String::new();
    for per_field in [2_000, 600, 200, 64] {
        out = scrub_json_value(value, per_field).to_string();
        if out.chars().count() <= max_chars {
            break;
        }
    }
    out
}

pub fn scrub_json_value(value: &Value, max_string_chars: usize) -> Value {
    match value {
        Value::String(s) => Value::String(scrub_and_truncate(s, max_string_chars)),
        Value::Array(arr) => Value::Array(
            arr.iter()
                .take(40)
                .map(|v| scrub_json_value(v, max_string_chars))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let lower = k.to_ascii_lowercase();
                if lower.contains("password")
                    || lower.contains("secret")
                    || lower.contains("api_key")
                    || lower.contains("apikey")
                    || lower == "authorization"
                    || lower == "cookie"
                {
                    out.insert(k.clone(), Value::String("[REDACTED]".into()));
                } else {
                    out.insert(k.clone(), scrub_json_value(v, max_string_chars));
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_common_secret_patterns_and_private_tags() {
        let raw = "key=sk-or-v1-1234567890abcdef1234567890abcdef \
                   ghp_abcdefghijklmnopqrstuv123456 \
                   OPENAI_API_KEY=supersecretvalue123 \
                   <private>top secret note</private> \
                   Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdef1234567890";
        let clean = redact_secrets(raw);
        assert!(!clean.contains("sk-or-v1-"));
        assert!(!clean.contains("ghp_abc"));
        assert!(!clean.contains("supersecretvalue123"));
        assert!(!clean.contains("top secret note"));
        assert!(!clean.contains("eyJhbGci"));
        assert!(clean.contains("[PRIVATE]"));
        assert!(clean.contains("[REDACTED_API_KEY]"));
    }

    #[test]
    fn redacts_api_key_headers_url_credentials_and_long_tokens() {
        let pg_scheme = "postgres";
        let pg_secret = "hunter2pass";
        let pg_cmd = format!("psql {pg_scheme}://admin:{pg_secret}@db:5432/app");
        let cases = [
            (
                r#"curl -H "x-api-key: abcd1234efgh5678" url"#,
                "abcd1234efgh5678",
            ),
            ("api-key: Zx9Qw8Er7Ty6Ui5O", "Zx9Qw8Er7Ty6Ui5O"),
            (pg_cmd.as_str(), pg_secret),
            (
                "git clone https://x-access-token:s3cr3tvalue@github.com/o/r",
                "s3cr3tvalue",
            ),
            (
                "HF_TOKEN=hf_abcdefghijklmnopqrstu",
                "hf_abcdefghijklmnopqrstu",
            ),
            (
                "export NPM_TOKEN='npm_1234567890abcdefXYZ'",
                "npm_1234567890abcdefXYZ",
            ),
        ];
        for (raw, secret) in cases {
            let clean = redact_secrets(raw);
            assert!(!clean.contains(secret), "{raw} -> {clean}");
            assert!(clean.contains("[REDACTED]"), "{raw} -> {clean}");
        }
        // Ordinary text and numeric limits are left alone.
        for raw in [
            "MAX_TOKEN=4096",
            "MAX_TOKENS=4096",
            "BUILD_TOKEN=12345678901234567890",
            "SHORT_TOKEN=abc123",
            "ssh://git@github.com/o/r.git",
            "see https://example.com/a:b@c for docs",
            "Content-Type: application/json",
        ] {
            assert_eq!(redact_secrets(raw), raw);
        }
        // Re-running over redacted text changes nothing.
        let repeat_pw = "bbbbbbbb";
        let once_input = format!("HF_TOKEN=hf_abcdefghijklmnopqrstu {pg_scheme}://a:{repeat_pw}@h");
        let once = redact_secrets(&once_input);
        assert_eq!(redact_secrets(&once), once);
    }

    #[test]
    fn bounded_json_stays_valid_and_within_limit() {
        let big = serde_json::json!({"input": {"command": "x".repeat(5_000)}, "output": ["y".repeat(5_000)]});
        let out = scrub_json_bounded(&big, 6_000);
        assert!(out.chars().count() <= 6_000, "{}", out.len());
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert!(
            parsed["input"]["command"]
                .as_str()
                .unwrap()
                .starts_with("xxx")
        );
    }

    #[test]
    fn truncates_utf8_safely() {
        let text = "🔥".repeat(500);
        let out = truncate_head_tail(&text, 120);
        assert!(out.contains("truncated"));
        assert!(out.chars().count() <= 130);
    }
}
