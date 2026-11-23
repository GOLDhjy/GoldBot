class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.10.7"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.7/goldbot-v0.10.7-macos-aarch64.tar.gz"
      sha256 "4c8945682c82db62d58ffe6112ea301b6f4cd88960db15dbdc4014e19ff3f56a"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.7/goldbot-v0.10.7-macos-x86_64.tar.gz"
      sha256 "efcdb48095a1d08e30ac268c44f97290feec49a3250556d64e7c46bdf18db7de"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.7/goldbot-v0.10.7-linux-x86_64.tar.gz"
    sha256 "412a5be6d828da3e4cf8cf2177c5d247041dcad88ac8b76736ee7a849b38cf12"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
