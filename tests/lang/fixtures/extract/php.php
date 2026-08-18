<?php
// Fixture docs mention doc_only_php and should not become code.
namespace Fixtures;

use App\Support\Helper;

interface GoldenRenderable {
    public function render(string $name): string;
}

class GoldenWidget {
    // Method docs mention doc_only_php.
    public function render(string $name): string {
        return format_widget($name);
    }
}

enum GoldenState {
    case Ready;
    case Spent;
}

function make_widget(string $name): GoldenWidget {
    return new GoldenWidget();
}

function format_widget(string $name): string {
    return trim($name);
}
