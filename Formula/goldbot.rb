class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.8.11"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.11/goldbot-v0.8.11-macos-aarch64.tar.gz"
      sha256 "cc0e4b7c1f4e0aa74f8d927c583731c0453fb4ffe021505ee44e8a321f04c247"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.11/goldbot-v0.8.11-macos-x86_64.tar.gz"
      sha256 "a18fea1a655a4e80dd529cd6e7f9f182f2a52e563c9fc00f325b94f85f5d44f0"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.11/goldbot-v0.8.11-linux-x86_64.tar.gz"
    sha256 "705ca2753214f0d27acfb694e27aa944df1bde633f88d2805cf66bf6b1872b0d"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
