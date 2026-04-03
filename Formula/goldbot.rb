class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.12"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.12/goldbot-v0.9.12-macos-aarch64.tar.gz"
      sha256 "d08a24faa699c510a3ee000e32d4d1464118e33c058203fd886b266d72438a6e"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.12/goldbot-v0.9.12-macos-x86_64.tar.gz"
      sha256 "de3afd50eba4e77ea92a3fef3069c057bdbd27407a2b4fc14c35a48835b24839"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.12/goldbot-v0.9.12-linux-x86_64.tar.gz"
    sha256 "039108e6a09862f1c42051fe9ff6b09e6cbf4ca8361b6125bafc833ffa3b8238"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
