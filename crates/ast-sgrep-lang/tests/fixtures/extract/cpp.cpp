// Fixture docs mention doc_only_cpp and should not become code.
#include <string>
#include "local.hpp"

namespace fixtures {

class GoldenWidget {
public:
    // Method docs mention doc_only_cpp.
    void render(const std::string& name) {
        helper(name);
    }
};

struct GoldenPoint {
    void move() {
        touch();
    }
};

enum class GoldenState {
    Ready,
    Spent
};

void helper(const std::string& name);
void touch();

void make_widget() {
    GoldenWidget w;
    w.render("x");
}

}  // namespace fixtures
