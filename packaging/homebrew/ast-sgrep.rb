# typed: strict
# frozen_string_literal: true

# Homebrew formula for the ast-sgrep command-line client.
class AstSgrep < Formula
  desc "Hybrid structural and semantic code search"
  homepage "https://github.com/AdityaVG13/ast-sgrep"
  # Homebrew tracks the latest published tag, not the workspace's unreleased version.
  url "https://github.com/AdityaVG13/ast-sgrep/archive/refs/tags/v1.1.0-alpha.tar.gz"
  version "1.1.0-alpha"
  sha256 "aaf34b409a3f21026548b236f568f77ea23dc26daf432847c46a678968f40c1b"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/ast-sgrep-cli")
  end

  def caveats
    <<~EOS
      This formula installs the asgrep CLI only. Editor integrations also require
      the asgrep-lsp binary, which can be installed from crates.io with:

        cargo install ast-sgrep-lsp
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/asgrep --version")
  end
end
