class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.6.6"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.6/goldbot-v0.6.6-macos-aarch64.tar.gz"
      sha256 "8997c28bc0ed63cadfb019446d12b389c283531b4d0a09ddf4329746f42c93c7"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.6/goldbot-v0.6.6-macos-x86_64.tar.gz"
      sha256 "cbd53c8638fb5fa606075c77a7a263a61534cf05354c7d518ef237d9e39b34f6"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.6/goldbot-v0.6.6-linux-x86_64.tar.gz"
    sha256 "52fcafe34962b57fd87ca9fbfd8a0cf1124b43cffbfd0587cb29089f4bedc5a2"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
