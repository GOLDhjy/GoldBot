class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.6.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.0/goldbot-v0.6.0-macos-aarch64.tar.gz"
      sha256 "cb100c421cd79b126fe6d2fc8e2bbab28185d93dd4650cf5176576d7be3ae456"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.0/goldbot-v0.6.0-macos-x86_64.tar.gz"
      sha256 "09a6516ac83a910c1e6aa2e676b08586b50511ec7ad0e048edceb858707e4fe5"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.6.0/goldbot-v0.6.0-linux-x86_64.tar.gz"
    sha256 "da0ee5b3eae8c54c2c91c61224d992dd50d636f36a02f293e142b2aabf183fcd"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
