class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.8.9"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.9/goldbot-v0.8.9-macos-aarch64.tar.gz"
      sha256 "3bb6b4337bfa0c1468e56ecea8ad4e8574dfac67b6b7ea1696fc06d37375872e"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.9/goldbot-v0.8.9-macos-x86_64.tar.gz"
      sha256 "11a80367c18165cd2c4ef7fd7a6bdcb938cc194d301c4b24ec14ae6be131929a"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.9/goldbot-v0.8.9-linux-x86_64.tar.gz"
    sha256 "f8a3752335cb184a5b1bd29fa2353aeb3dacfd6c16a8c8520cdf1fecc1693ba3"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
