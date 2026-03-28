class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.5"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.5/goldbot-v0.9.5-macos-aarch64.tar.gz"
      sha256 "8516ac3cb1b67b566626c48d1272795391191d1a4cab31015dcdec8b648cbf6d"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.5/goldbot-v0.9.5-macos-x86_64.tar.gz"
      sha256 "ab7a6bdfa8e3c4e3f3d47ef28c3e50111a444bb20997b97e4deb4ccc2586f459"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.5/goldbot-v0.9.5-linux-x86_64.tar.gz"
    sha256 "d364f9bd8906c294bf9b919e770a20701358337411cfa973297d4d3a7cc8b451"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
