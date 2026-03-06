class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.2"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.2/goldbot-v0.9.2-macos-aarch64.tar.gz"
      sha256 "15a211dfd6157ed143bcc82e3a58a6e87c2413f3311ee8deead826c974f038fd"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.2/goldbot-v0.9.2-macos-x86_64.tar.gz"
      sha256 "f82dfd0b59417dd3ee532c28aaf34bb644be77637082e57749a4188377950a39"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.2/goldbot-v0.9.2-linux-x86_64.tar.gz"
    sha256 "b49d56bd3faa759cc73ab2993bb9a6009a39bf08638000d222a0bf1abf2ccef0"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
