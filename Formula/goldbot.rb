class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.7.12"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.12/goldbot-v0.7.12-macos-aarch64.tar.gz"
      sha256 "249b6f34d819100acedb0c5ec9a4633b322fedc05a88cf2c7470e9f5e0133d8a"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.12/goldbot-v0.7.12-macos-x86_64.tar.gz"
      sha256 "54bd8f00ba62820300d7ec21df318899ebcf22f01ab2fe116eefd095583d42de"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.12/goldbot-v0.7.12-linux-x86_64.tar.gz"
    sha256 "4b01b41419d6d39bb1ca3ada2362706a3864a27d42cda0cf60c839bc74d12d06"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
