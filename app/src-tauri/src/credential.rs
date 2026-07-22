const SERVICE: &str = "dev.kzheart.ferry";
const DEEPSEEK_ACCOUNT: &str = "deepseek-api-key";

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, DEEPSEEK_ACCOUNT).map_err(|_| "无法访问系统凭据存储".to_owned())
}

fn validate_api_key(api_key: &str) -> Result<&str, String> {
    let value = api_key.trim();
    if !(16..=512).contains(&value.len())
        || value.chars().any(|character| character.is_control())
        || value.chars().any(char::is_whitespace)
    {
        return Err("DeepSeek API Key 格式无效".to_owned());
    }
    Ok(value)
}

pub(crate) fn load_deepseek_api_key() -> Option<String> {
    entry()
        .ok()?
        .get_password()
        .ok()
        .filter(|value| !value.is_empty())
}

#[tauri::command]
pub(crate) fn agent_credential_status() -> bool {
    std::env::var("DEEPSEEK_API_KEY")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
        || load_deepseek_api_key().is_some()
}

#[tauri::command]
pub(crate) fn agent_credential_set(api_key: String) -> Result<(), String> {
    let value = validate_api_key(&api_key)?;
    entry()?
        .set_password(value)
        .map_err(|_| "无法保存 DeepSeek 凭据".to_owned())?;
    crate::agent::reset();
    Ok(())
}

#[tauri::command]
pub(crate) fn agent_credential_clear() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(_) => return Err("无法清除 DeepSeek 凭据".to_owned()),
    }
    crate::agent::reset();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_api_key;

    #[test]
    fn api_key_validation_never_accepts_whitespace_or_controls() {
        assert!(validate_api_key("sk-1234567890abcdef").is_ok());
        assert!(validate_api_key("short").is_err());
        assert!(validate_api_key("sk-1234567890 abcdef").is_err());
        assert!(validate_api_key("sk-1234567890\nabcdef").is_err());
    }
}
