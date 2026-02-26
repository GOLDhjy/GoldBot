class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.8.2"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.2/goldbot-v0.8.2-macos-aarch64.tar.gz"
      sha256 "8fdffbd3536b0442ae57a97ad7802aa6e3b584b9cb0c5efa3ae4370bb6fa4ee7"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.2/goldbot-v0.8.2-macos-x86_64.tar.gz"
      sha256 "3987bc2a10f9da6d2ce9ab0825c4e90b6fb69eb397ab5c8f3d238864fe2ed287"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.2/goldbot-v0.8.2-linux-x86_64.tar.gz"
    sha256 "4476557742e6864efd5f9414e32522a2ef44f3aa8f5978abbbe93658aec894f0"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
