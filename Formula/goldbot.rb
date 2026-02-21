class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.3.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.3.0/goldbot-v0.3.0-macos-aarch64.tar.gz"
      sha256 "c4bdaba71a207fa4acbe0892380d97ab83dde7e412d56dd1b35c85e3e66ac036"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.3.0/goldbot-v0.3.0-macos-x86_64.tar.gz"
      sha256 "9b32e082fb4735b17b549561235def1a481761ec6212fc8b4a6c2e7aafceb369"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.3.0/goldbot-v0.3.0-linux-x86_64.tar.gz"
    sha256 "137e9dcf2ee68560d4a818ead86e9eb7dcb8340663f3cf2a3b73abc8891a18b9"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
