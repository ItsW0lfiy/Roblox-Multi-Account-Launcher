pub fn validate(input: &str) -> Result<String, String> {
    let value = input.trim();
    if value.is_empty() {
        return Ok("roblox:".into());
    }
    let lower = value.to_ascii_lowercase();
    if lower.contains(".roblosecurity")
        || lower.contains("authenticationticket=")
        || lower.contains("ticket=")
        || lower.contains("token=")
        || lower.contains("password=")
    {
        return Err("The link appears to contain authentication data and was refused.".into());
    }
    if lower.starts_with("roblox:") || lower.starts_with("roblox-player:") {
        return Ok(value.into());
    }
    let Some(rest) = lower.strip_prefix("https://") else {
        return Err(
            "Use a Roblox HTTPS game/share link or a supported Roblox protocol link.".into(),
        );
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host == "roblox.com" || host.ends_with(".roblox.com") {
        Ok(value.into())
    } else {
        Err("Only links hosted by roblox.com are accepted.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_roblox_links_and_rejects_credentials_or_foreign_hosts() {
        assert!(validate("").is_ok());
        assert!(validate("roblox://placeId=123").is_ok());
        assert!(validate("https://www.roblox.com/games/123/example").is_ok());
        assert!(validate("https://evil.example/games/123").is_err());
        assert!(validate("roblox:?authenticationTicket=secret").is_err());
    }
}
