class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.7.1"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.1/goldbot-v0.7.1-macos-aarch64.tar.gz"
      sha256 "37d1e1c1d3e1b297b782cf8b0c433a9f8caf6e02e449ec085807d2a22af52db9"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.1/goldbot-v0.7.1-macos-x86_64.tar.gz"
      sha256 "23f3e3eeaed739c6573650a5b469063415d3fe7d62dfe70bc47f1a20ff9b897b"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.1/goldbot-v0.7.1-linux-x86_64.tar.gz"
    sha256 "f6b36ff5ecd4c41b131312af1ef86e87f3d45ee447f081710479c139df7fb94f"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
