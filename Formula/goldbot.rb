class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.20"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.20/goldbot-v0.9.20-macos-aarch64.tar.gz"
      sha256 "869b7db524142324b6882157b340ffdc3df30ee1661aa982cb3e9034ab981b54"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.20/goldbot-v0.9.20-macos-x86_64.tar.gz"
      sha256 "bdabdcd82e4fc6c2dfd96ae0c71b34fe5f320d8b3772548bbfef62c00b3cb79d"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.20/goldbot-v0.9.20-linux-x86_64.tar.gz"
    sha256 "37fa8cdc6439b0ba83067f1e13016f17a69ba4591844f3bba7c6c50bf0249f2a"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
