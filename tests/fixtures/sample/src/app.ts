export function main() {
  processRequest("hello");
  authRefresh();
}

export function processRequest(input: string): string {
  validateInput(input);
  return `processed: ${input}`;
}

function validateInput(input: string) {
  if (!input) {
    throw new Error("empty input");
  }
}

export function authRefresh() {
  const token = fetchToken();
  storeToken(token);
}

function fetchToken(): string {
  return "token";
}

function storeToken(token: string) {
  console.log(token);
}
