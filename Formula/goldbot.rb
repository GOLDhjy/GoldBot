class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.3"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.3/goldbot-v0.9.3-macos-aarch64.tar.gz"
      sha256 "6637088a2b5c0dc5917db359645003fb0111f47000e3433d4ebc6fc4a7f44814"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.3/goldbot-v0.9.3-macos-x86_64.tar.gz"
      sha256 "c413e06dab80853a1bb735b4bb471a72bafff526db4ec4b6b04da517629f3a56"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.3/goldbot-v0.9.3-linux-x86_64.tar.gz"
    sha256 "bde8baf4458e1a6a959a3250b2692174520deacd18524fed1273330d4b3080bf"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
