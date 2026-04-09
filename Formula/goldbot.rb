class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.13"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.13/goldbot-v0.9.13-macos-aarch64.tar.gz"
      sha256 "1a4521c909f6476647a4be2d0f7735ff4b58d90e801ba088fa9903e2805c6d6e"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.13/goldbot-v0.9.13-macos-x86_64.tar.gz"
      sha256 "9b6f538477de03fc8ac308d9a563c8adc3641bcf4d33c515a6a9125ad621bf85"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.13/goldbot-v0.9.13-linux-x86_64.tar.gz"
    sha256 "6c6688f3c547c8f05e554f431cea141ae48afec35b5ffed3c48518262b36a023"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
