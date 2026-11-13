class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.10.5"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.5/goldbot-v0.10.5-macos-aarch64.tar.gz"
      sha256 "fa937283a31287d5dededf8e8a75b4aae1e7930adc96af7ec479c1312996bbc9"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.5/goldbot-v0.10.5-macos-x86_64.tar.gz"
      sha256 "f28032fcff4c7b1da877d0779a3095e3bae949a0b637d0f0b8dfd84f47c56312"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.5/goldbot-v0.10.5-linux-x86_64.tar.gz"
    sha256 "0c7d9229d0c3d74a340f845bf9b30704c68c01dd0afc80da96f05f443883f6d1"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
