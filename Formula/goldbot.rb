class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.6.1"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.1/goldbot-v0.6.1-macos-aarch64.tar.gz"
      sha256 "c0e3e7ac9df4c77a622c36ba760a652fc2683a1e0296da1a8c7c6f4a62ebe5b8"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.1/goldbot-v0.6.1-macos-x86_64.tar.gz"
      sha256 "1d36b4f36c55377fcda1d5ecc1804681051e224bf08a0da47ff9c0e531e92d56"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.1/goldbot-v0.6.1-linux-x86_64.tar.gz"
    sha256 "86a7c98bfe81357684ce507ea05406b9b487d65bbd79395ca8908fd8717131c0"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
