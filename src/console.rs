use std::{borrow::Cow, fs, path::Path};

use nix::unistd;
use simple_error::try_with;

use crate::{attach::AttachOptions, result::Result};

fn whitelisted(ch: char) -> bool {
    matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '=' | '/' | ',' | '.' | '+')
}

/// Escape characters that may have special meaning in a shell, including spaces.
pub fn shell_escape(s: Cow<str>) -> Cow<str> {
    if !s.is_empty() && s.contains(whitelisted) {
        return s;
    }

    let mut es = String::with_capacity(s.len() + 2);
    es.push('\'');
    for ch in s.chars() {
        match ch {
            '\'' | '!' => {
                es.push_str("'\\");
                es.push(ch);
                es.push('\'');
            }
            _ => es.push(ch),
        }
    }
    es.push('\'');
    es.into()
}

pub fn console(attach: &AttachOptions) -> Result<()> {
    // Does this need to be portable?
    let res = try_with!(
        fs::read_link(Path::new("/proc/self/fd/0")),
        "Cannot open stdin"
    );
    println!("Run the following command in a different terminal");
    let mut attach_cmd = vec![format!(
        "vmsh attach --pts {} --backing-file {} {} --",
        res.as_path().display(),
        attach.backing.display(),
        attach.pid
    )];
    for arg in &attach.command[1..] {
        attach_cmd.push(shell_escape(arg.into()).to_string())
    }
    println!("{}", attach_cmd.join(" "));
    unistd::pause();
    Ok(())
}
