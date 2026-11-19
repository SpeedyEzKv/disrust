/// Discord's default avatar index for migrated and legacy usernames.
pub fn default_avatar_index(user_id: &str, discriminator: &str) -> Option<u8> {
    if !digits(user_id, 20) || !digits(discriminator, 4) {
        return None;
    }
    let id = user_id.parse::<u64>().ok()?;
    if discriminator == "0" {
        Some(((id >> 22) % 6) as u8)
    } else {
        Some((discriminator.parse::<u16>().ok()? % 5) as u8)
    }
}

fn digits(value: &str, max_len: usize) -> bool {
    !value.is_empty() && value.len() <= max_len && value.bytes().all(|byte| byte.is_ascii_digit())
}
