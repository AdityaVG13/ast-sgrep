import os

def main():
    process_request("hello")
    auth_refresh()


def process_request(input: str) -> str:
    validate_input(input)
    return f"processed: {input}"


def validate_input(input: str):
    if not input:
        raise ValueError("empty input")


def auth_refresh():
    token = fetch_token()
    store_token(token)


def fetch_token() -> str:
    return "token"


def store_token(token: str):
    _ = token
