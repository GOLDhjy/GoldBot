class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.7.9"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.9/goldbot-v0.7.9-macos-aarch64.tar.gz"
      sha256 "96ddfa961e1333452eea37ad703d2410c38bd3c0f80fd55716165179e3cdc2aa"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.9/goldbot-v0.7.9-macos-x86_64.tar.gz"
      sha256 "25e08b722b14ecf5efadaf78d2ec701a9bdd144cebe64dab3a6044d9e328f6a7"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.9/goldbot-v0.7.9-linux-x86_64.tar.gz"
    sha256 "e2577308c858188ffc456e9f5abb2f7ff3cd05fbaf8a2d0da36ab08f7e38f166"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
