class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.7.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.0/goldbot-v0.7.0-macos-aarch64.tar.gz"
      sha256 "80381b963b5f7fc2a3d219a46cb726c6face779d529f0f9385581575100be177"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.0/goldbot-v0.7.0-macos-x86_64.tar.gz"
      sha256 "01763278c0d4fd55eb826febaa9d1dbef4c379465d6795587cba18a077be6472"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.0/goldbot-v0.7.0-linux-x86_64.tar.gz"
    sha256 "c217b0c04269fea84ce6682842b76f054881d67ebb385c1274c93b97d4c40365"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
