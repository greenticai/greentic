use std::io::{self, BufRead, IsTerminal, Write};

use gtc::error::{GtcError, GtcResult};
use zeroize::Zeroizing;

pub(super) fn can_prompt_interactively() -> bool {
    if cfg!(test) {
        return false;
    }
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

pub(super) fn prompt_choice(prompt: &str, options: &[&str]) -> GtcResult<usize> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    prompt_choice_with_io(&mut reader, &mut writer, prompt, options)
}

pub(super) fn parse_prompt_choice(choice: usize, options_len: usize) -> GtcResult<usize> {
    if choice == 0 || choice > options_len {
        return Err(GtcError::message("invalid selection"));
    }
    Ok(choice - 1)
}

pub(super) fn prompt_non_empty(prompt: &str) -> GtcResult<String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    prompt_non_empty_with_io(&mut reader, &mut writer, prompt)
}

fn prompt_non_empty_with_io<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    prompt: &str,
) -> GtcResult<String> {
    loop {
        let value = prompt_optional_with_io(reader, writer, prompt)?;
        if let Some(value) = value {
            return Ok(value);
        }
        writeln!(writer, "A value is required.")
            .map_err(|err| GtcError::message(err.to_string()))?;
    }
}

pub(super) fn prompt_optional(prompt: &str) -> GtcResult<Option<String>> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    prompt_optional_with_io(&mut reader, &mut writer, prompt)
}

fn prompt_optional_with_io<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    prompt: &str,
) -> GtcResult<Option<String>> {
    write!(writer, "{prompt} ").map_err(|err| GtcError::message(err.to_string()))?;
    writer
        .flush()
        .map_err(|err| GtcError::message(err.to_string()))?;
    let mut input = String::new();
    reader
        .read_line(&mut input)
        .map_err(|err| GtcError::message(err.to_string()))?;
    Ok(normalize_prompt_input(&input))
}

pub(super) fn prompt_value_with_default(prompt: &str, default: Option<&str>) -> GtcResult<String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    prompt_value_with_default_with_io(&mut reader, &mut writer, prompt, default)
}

fn prompt_value_with_default_with_io<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    prompt: &str,
    default: Option<&str>,
) -> GtcResult<String> {
    loop {
        match default {
            Some(default) => {
                write!(writer, "{prompt} [{default}] ")
                    .map_err(|err| GtcError::message(err.to_string()))?;
            }
            None => {
                write!(writer, "{prompt} ").map_err(|err| GtcError::message(err.to_string()))?;
            }
        }
        writer
            .flush()
            .map_err(|err| GtcError::message(err.to_string()))?;
        let mut input = String::new();
        reader
            .read_line(&mut input)
            .map_err(|err| GtcError::message(err.to_string()))?;
        if let Some(value) = resolve_prompt_value(&input, default) {
            return Ok(value);
        }
        writeln!(writer, "A value is required.")
            .map_err(|err| GtcError::message(err.to_string()))?;
    }
}

pub(super) fn prompt_secret(prompt: &str) -> GtcResult<Zeroizing<String>> {
    loop {
        let value =
            rpassword::prompt_password(prompt).map_err(|err| GtcError::message(err.to_string()))?;
        if let Some(value) = normalize_secret_value(value) {
            return Ok(value);
        }
        println!("A value is required.");
    }
}

pub(super) fn prompt_optional_secret(prompt: &str) -> GtcResult<Option<Zeroizing<String>>> {
    let value =
        rpassword::prompt_password(prompt).map_err(|err| GtcError::message(err.to_string()))?;
    Ok(normalize_secret_value(value))
}

fn normalize_prompt_input(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn resolve_prompt_value(input: &str, default: Option<&str>) -> Option<String> {
    normalize_prompt_input(input).or_else(|| default.map(|value| value.to_string()))
}

fn normalize_secret_value(value: String) -> Option<Zeroizing<String>> {
    if value.trim().is_empty() {
        None
    } else {
        Some(Zeroizing::new(value))
    }
}

fn prompt_choice_with_io<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    prompt: &str,
    options: &[&str],
) -> GtcResult<usize> {
    writeln!(writer, "{prompt}").map_err(|err| GtcError::message(err.to_string()))?;
    for (idx, option) in options.iter().enumerate() {
        writeln!(writer, "{} ) {}", idx + 1, option)
            .map_err(|err| GtcError::message(err.to_string()))?;
    }
    write!(writer, "> ").map_err(|err| GtcError::message(err.to_string()))?;
    writer
        .flush()
        .map_err(|err| GtcError::message(err.to_string()))?;
    let mut input = String::new();
    reader
        .read_line(&mut input)
        .map_err(|err| GtcError::message(err.to_string()))?;
    let choice = input
        .trim()
        .parse::<usize>()
        .map_err(|_| GtcError::message("invalid selection"))?;
    parse_prompt_choice(choice, options.len())
}

#[cfg(test)]
mod tests {
    use super::{
        can_prompt_interactively, normalize_prompt_input, normalize_secret_value,
        parse_prompt_choice, prompt_choice_with_io, prompt_non_empty_with_io,
        prompt_optional_with_io, prompt_value_with_default_with_io, resolve_prompt_value,
    };
    use std::io::Cursor;

    #[test]
    fn can_prompt_interactively_is_disabled_under_tests() {
        assert!(!can_prompt_interactively());
    }

    #[test]
    fn parse_prompt_choice_rejects_out_of_range_values() {
        assert_eq!(
            parse_prompt_choice(0, 2).unwrap_err().to_string(),
            "invalid selection"
        );
        assert_eq!(
            parse_prompt_choice(3, 2).unwrap_err().to_string(),
            "invalid selection"
        );
    }

    #[test]
    fn parse_prompt_choice_accepts_last_value() {
        assert_eq!(parse_prompt_choice(3, 3).expect("choice"), 2);
    }

    #[test]
    fn normalize_prompt_input_trims_non_empty_values() {
        assert_eq!(
            normalize_prompt_input("  demo value \n"),
            Some("demo value".to_string())
        );
        assert_eq!(normalize_prompt_input("   \n"), None);
    }

    #[test]
    fn resolve_prompt_value_uses_default_when_input_blank() {
        assert_eq!(
            resolve_prompt_value(" \n", Some("fallback")),
            Some("fallback".to_string())
        );
        assert_eq!(
            resolve_prompt_value(" chosen ", Some("fallback")),
            Some("chosen".to_string())
        );
        assert_eq!(resolve_prompt_value(" \n", None), None);
    }

    #[test]
    fn normalize_secret_value_rejects_blank_values() {
        assert!(normalize_secret_value(" \n".to_string()).is_none());
        let value = normalize_secret_value("secret".to_string()).expect("secret");
        assert_eq!(value.as_str(), "secret");
    }

    #[test]
    fn prompt_choice_with_io_reads_and_parses_selection() {
        let mut input = Cursor::new(b"2\n".to_vec());
        let mut output = Vec::new();
        let choice =
            prompt_choice_with_io(&mut input, &mut output, "Pick", &["A", "B"]).expect("choice");
        assert_eq!(choice, 1);
        let rendered = String::from_utf8(output).expect("utf8");
        assert!(rendered.contains("Pick"));
        assert!(rendered.contains("1 ) A"));
    }

    #[test]
    fn prompt_optional_with_io_trims_values() {
        let mut input = Cursor::new(b" demo \n".to_vec());
        let mut output = Vec::new();
        let value = prompt_optional_with_io(&mut input, &mut output, "Value:").expect("optional");
        assert_eq!(value.as_deref(), Some("demo"));
    }

    #[test]
    fn prompt_non_empty_with_io_retries_after_blank_input() {
        let mut input = Cursor::new(b"\nchosen\n".to_vec());
        let mut output = Vec::new();
        let value = prompt_non_empty_with_io(&mut input, &mut output, "Value:").expect("value");
        assert_eq!(value, "chosen");
        let rendered = String::from_utf8(output).expect("utf8");
        assert!(rendered.contains("A value is required."));
    }

    #[test]
    fn prompt_value_with_default_with_io_uses_default_after_blank_input() {
        let mut input = Cursor::new(b"\n".to_vec());
        let mut output = Vec::new();
        let value = prompt_value_with_default_with_io(
            &mut input,
            &mut output,
            "Region:",
            Some("us-central1"),
        )
        .expect("value");
        assert_eq!(value, "us-central1");
    }
}
