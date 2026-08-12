/* Fixture docs mention doc_only_c and should not become code. */
#include <stdio.h>
#include "local.h"

struct GoldenWidget {
    int x;
};

enum GoldenState {
    Ready,
    Spent
};

typedef struct GoldenWidget GoldenAlias;

/* Function docs mention doc_only_c. */
void helper(const char *name);

void render(const char *name) {
    helper(name);
}

void format_widget(const char *name) {
    render(name);
}
