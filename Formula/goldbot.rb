class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.7"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.7/goldbot-v0.9.7-macos-aarch64.tar.gz"
      sha256 "178d9d49b7eed9f37e48f384521414e66a2225b62a5f7e55368c484253f9da0b"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.7/goldbot-v0.9.7-macos-x86_64.tar.gz"
      sha256 "1f86f4f606fc66bc815629fc53c580e343d4e8de77a80f7347904ff7d1a3d4ec"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.7/goldbot-v0.9.7-linux-x86_64.tar.gz"
    sha256 "a980989f31bb2cacf11cdabde3f6ac66c447be75eb4bf791b066346ba7d772d3"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
