class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.1"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.1/goldbot-v0.9.1-macos-aarch64.tar.gz"
      sha256 "4fd42e507c2020628a108c3cfec07d3155bf7913fc9e6fa383b5c27f0ca6515a"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.1/goldbot-v0.9.1-macos-x86_64.tar.gz"
      sha256 "18f2a4e6f1a6bafb38c020049ffc37714fa037c6d25ac19f09bc5e33fe72ead4"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.1/goldbot-v0.9.1-linux-x86_64.tar.gz"
    sha256 "a6f751384d928d36ce9c3bc33aee16eafab3b1868ade80dc68995fb6a97feff7"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
