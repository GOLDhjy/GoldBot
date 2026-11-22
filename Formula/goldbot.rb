class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.10.6"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.6/goldbot-v0.10.6-macos-aarch64.tar.gz"
      sha256 "a76c843e63ee701e746fd5f0ab27cf7d6e083c0cf933d00e7f4ebbd0239bca0f"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.6/goldbot-v0.10.6-macos-x86_64.tar.gz"
      sha256 "a895d487ea179baf9dbc87e2ba56c5aa2844a722a3fed7ac2e6b3755d6dad659"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.10.6/goldbot-v0.10.6-linux-x86_64.tar.gz"
    sha256 "eff3914779a86257731c5f90a2be8128394fa2dd16c6a529b0a0925b5a49cc70"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
