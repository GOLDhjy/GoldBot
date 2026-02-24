class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.7.5"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.5/goldbot-v0.7.5-macos-aarch64.tar.gz"
      sha256 "c4a52f85d6e03e2b67a684573cbabd58ff99a1bd9b82ce471ce97840210b0ec7"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.5/goldbot-v0.7.5-macos-x86_64.tar.gz"
      sha256 "8c1f22e79e676e27519a31aa1b05d57dab4463f53918b258a1f7977673ac355d"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.5/goldbot-v0.7.5-linux-x86_64.tar.gz"
    sha256 "c9f3babbbd1ef8a3beaa5cb3b5ffb3f46c06151108802b836bd8bdea7874aad1"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
