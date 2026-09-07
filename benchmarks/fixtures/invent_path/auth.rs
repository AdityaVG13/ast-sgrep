pub fn auth_refresh() {
    let token = fetch_token();
    store_token(token);
}

fn fetch_token() -> String {
    String::from("ok")
}

fn store_token(_token: String) {}
