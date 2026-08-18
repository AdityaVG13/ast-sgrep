require "json"

def main
  process_request("hello")
  auth_refresh
end

def process_request(input)
  validate_input(input)
  "processed: #{input}"
end

def validate_input(input)
  raise "empty" if input.empty?
end

def auth_refresh
  token = fetch_token
  store_token(token)
end

def fetch_token
  "token"
end

def store_token(token)
  # store
end

main
