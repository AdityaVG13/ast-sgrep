use std::collections::HashMap;

fn main() {
    let _ = process_request("hello");
    auth_refresh();
}

fn process_request(input: &str) -> String {
    validate_input(input);
    format!("processed: {input}")
}

fn validate_input(input: &str) {
    if input.is_empty() {
        panic!("empty input");
    }
}

fn auth_refresh() {
    let token = fetch_token();
    store_token(token);
}

fn fetch_token() -> String {
    "token".to_string()
}

fn store_token(token: String) {
    let _ = token;
}
