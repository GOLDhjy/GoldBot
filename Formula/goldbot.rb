class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.8.1"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.1/goldbot-v0.8.1-macos-aarch64.tar.gz"
      sha256 "246f1da6aa065d367dd3d08e8d65612a7e96c074ec5940e33a5f54e28c48619d"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.1/goldbot-v0.8.1-macos-x86_64.tar.gz"
      sha256 "e4afbc8bb09f581f404cf0104e27a8c8e108946518ad5202fa1932be26302193"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.1/goldbot-v0.8.1-linux-x86_64.tar.gz"
    sha256 "ecf56b27458c1b264f7d210f6c27bd3dc5c972fc1e9ba5f9a6c0abbb4a1738ba"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
