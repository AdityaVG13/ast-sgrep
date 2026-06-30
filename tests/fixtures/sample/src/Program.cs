using System;

public class Program {
    public static void Main(string[] args) {
        ProcessRequest("hello");
        AuthRefresh();
    }

    public static string ProcessRequest(string input) {
        ValidateInput(input);
        return $"processed: {input}";
    }

    public static void ValidateInput(string input) {
        if (string.IsNullOrEmpty(input)) {
            throw new ArgumentException("empty");
        }
    }

    public static void AuthRefresh() {
        var token = FetchToken();
        StoreToken(token);
    }

    public static string FetchToken() {
        return "token";
    }

    public static void StoreToken(string token) {
        // store
    }
}
