class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.7.7"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.7/goldbot-v0.7.7-macos-aarch64.tar.gz"
      sha256 "e5282f09dee88dc2ea3282ef9a0cf86d9c5e3c196e0f3dedea1b8066d149b683"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.7/goldbot-v0.7.7-macos-x86_64.tar.gz"
      sha256 "b794e4207b3e624a2fe3732d192fcd4f36dbbbb3c0c8002c6dfb81738ccb4ee0"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.7/goldbot-v0.7.7-linux-x86_64.tar.gz"
    sha256 "1c2567e94df01829b84c7abd57d012c56f1859c9b2ea3c0327952364cfd35faa"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
