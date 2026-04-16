class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.15"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.15/goldbot-v0.9.15-macos-aarch64.tar.gz"
      sha256 "cf99c569f02fe4e260982a132f0325e02d22bc6bf66868d63928e111eb3ad2fa"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.15/goldbot-v0.9.15-macos-x86_64.tar.gz"
      sha256 "664588768cc7d1731b2dd042e8963982b2add7ef6efcc25152f0258a6c247ee5"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.15/goldbot-v0.9.15-linux-x86_64.tar.gz"
    sha256 "dadb0026499fd374924f92fa0a945c1db875e5a9d05dac006de3c6e60db85340"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
