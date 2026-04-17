class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.17"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.17/goldbot-v0.9.17-macos-aarch64.tar.gz"
      sha256 "22e59b9d126fc27ae1bed7b551a984c5d799442827c8d5d9c4181dfa8f9baddb"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.17/goldbot-v0.9.17-macos-x86_64.tar.gz"
      sha256 "8404d6181ce7b5d941719aa618c64133fc8d4d405a06229cc02f9587a6bce41c"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.17/goldbot-v0.9.17-linux-x86_64.tar.gz"
    sha256 "eec24a80f3f7d180f400812ff219096b74e7ac70f02925d9e7e7b56a198dbeaf"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
