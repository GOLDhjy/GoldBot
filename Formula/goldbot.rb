class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.5.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.5.0/goldbot-v0.5.0-macos-aarch64.tar.gz"
      sha256 "20d6694d2208a29634bacb3a5a93991e654b7f8165df671ad690fa509cafb735"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.5.0/goldbot-v0.5.0-macos-x86_64.tar.gz"
      sha256 "69dcad0d42360eb0ab91049ddb9a75712b47329410d1b31db5f222ad0d01a75d"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.5.0/goldbot-v0.5.0-linux-x86_64.tar.gz"
    sha256 "60798204183965b9ca129f0625baa555238a4038d524045eefa9dcf8427aa897"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
