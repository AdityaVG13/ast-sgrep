using System.Text;

namespace Fixtures {
    /// <summary>Class docs mention DocOnlyCSharp and should not become code.</summary>
    [System.Obsolete]
    public class GoldenWidget {
        public string Name { get; init; }

        public GoldenWidget() {
            Helper("constructor");
        }

        /// <summary>Method docs mention DocOnlyCSharp.</summary>
        public string Render(string name) {
            var normalized = Helper(name);
            return normalized;
        }

        public string Echo(string value) => value;

        private static string Helper(string name) {
            return name.Trim();
        }
    }

    public struct GoldenPoint {
        public void Move() {
            Local();
            void Local() { Touch(); }
        }

        private static void Touch() { }
    }

    public record GoldenRecord(string Name);

    public enum GoldenState {
        Ready,
        Spent
    }
}
