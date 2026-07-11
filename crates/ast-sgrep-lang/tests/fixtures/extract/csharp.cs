using System.Text;

namespace Fixtures {
    /// <summary>Class docs mention DocOnlyCSharp and should not become code.</summary>
    public class GoldenWidget {
        /// <summary>Method docs mention DocOnlyCSharp.</summary>
        public string Render(string name) {
            return Helper(name);
        }

        private static string Helper(string name) {
            return name.Trim();
        }
    }
}
