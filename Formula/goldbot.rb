class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.7.6"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.6/goldbot-v0.7.6-macos-aarch64.tar.gz"
      sha256 "2ce04cb51629cea8f981624bb27d101e233edec5fc4a33849d25f4616e10f619"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.6/goldbot-v0.7.6-macos-x86_64.tar.gz"
      sha256 "eca70e37dff764a9a2a9e152c960e009685146f645e6dbc3aedf1d4d04676ae9"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.6/goldbot-v0.7.6-linux-x86_64.tar.gz"
    sha256 "8bcee0b41e4ab227691400cfbbac0c48e0345ff80a5eaeb97fc4084fc1129dfa"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
