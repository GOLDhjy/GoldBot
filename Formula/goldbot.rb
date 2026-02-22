class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.6.2"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.2/goldbot-v0.6.2-macos-aarch64.tar.gz"
      sha256 "04d7d860abadca69cf3c9e13844cea461478734e372e39193a41a88f8e9ce8cc"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.2/goldbot-v0.6.2-macos-x86_64.tar.gz"
      sha256 "05056540d23b20f8ae5f35bcf05ee4875b3ec56efde3f06071eeb7d175f5ba08"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.2/goldbot-v0.6.2-linux-x86_64.tar.gz"
    sha256 "58cb7abf8bf39649009b4f6ff5250a0b98a82b1370b2075e17f7559a8fd54b9d"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
