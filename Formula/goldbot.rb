class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.1.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.1.0/goldbot-v0.1.0-macos-aarch64.tar.gz"
      sha256 ""
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.1.0/goldbot-v0.1.0-macos-x86_64.tar.gz"
      sha256 ""
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.1.0/goldbot-v0.1.0-linux-x86_64.tar.gz"
    sha256 ""
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
