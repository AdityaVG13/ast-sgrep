public class Main {
    public static void main(String[] args) {
        processRequest("hello");
        authRefresh();
    }

    public static String processRequest(String input) {
        validateInput(input);
        return "processed: " + input;
    }

    public static void validateInput(String input) {
        if (input.isEmpty()) {
            throw new IllegalArgumentException("empty");
        }
    }

    public static void authRefresh() {
        String token = fetchToken();
        storeToken(token);
    }

    public static String fetchToken() {
        return "token";
    }

    public static void storeToken(String token) {
        // store
    }
}
