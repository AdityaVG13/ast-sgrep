pub fn rate_limit_client(client_id: &str) -> bool {
    !client_id.is_empty()
}
