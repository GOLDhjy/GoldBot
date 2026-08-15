class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.10.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.0/goldbot-v0.10.0-macos-aarch64.tar.gz"
      sha256 "e059c40f446b57adc7668aa328a7165ee8f682efda36cd6c7c6055343332698c"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.0/goldbot-v0.10.0-macos-x86_64.tar.gz"
      sha256 "322c89add21c3b142303bf0e534cab55d15d6c24bffe48f510c237e04639314c"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.0/goldbot-v0.10.0-linux-x86_64.tar.gz"
    sha256 "96b0fa4a272ce9d22746ee8c73034420460f26332fdc8f58d7bebb4c613a8989"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
