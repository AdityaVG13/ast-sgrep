# typed: strict
# frozen_string_literal: true

# Homebrew formula for the ast-sgrep command-line client.
class AstSgrep < Formula
  desc "Hybrid structural and semantic code search"
  homepage "https://github.com/AdityaVG13/ast-sgrep"
  url "https://github.com/AdityaVG13/ast-sgrep/archive/refs/tags/v1.0.0-alpha.tar.gz"
  version "1.0.0-alpha"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/ast-sgrep-cli")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/asgrep --version")
  end
end
