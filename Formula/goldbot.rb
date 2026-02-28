class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.8.10"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.10/goldbot-v0.8.10-macos-aarch64.tar.gz"
      sha256 "c8ecb8cc4ba7b1a858d3dd41ecc2416601205c9f4d6b0bb91af7dbf7bbcd0c7d"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.10/goldbot-v0.8.10-macos-x86_64.tar.gz"
      sha256 "37566d842f9b74ca66f4018ade9adf2b38b1e2f87d8d450bc6632154713885d9"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.10/goldbot-v0.8.10-linux-x86_64.tar.gz"
    sha256 "a93bbfd2f63254a576fb14f42afc938e54a1e26fcab9398747bb92312fccfdf9"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
