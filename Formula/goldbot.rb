class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.7.11"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.11/goldbot-v0.7.11-macos-aarch64.tar.gz"
      sha256 "dc5c1b1c8c0a04ada3fe89c161bf6d127c25cb23890ed459e989360235f8f45f"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.11/goldbot-v0.7.11-macos-x86_64.tar.gz"
      sha256 "14be7258cfa1164cd00eab24844f67d8c9b1055689652d22ed75a6a7e6d2d54f"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.7.11/goldbot-v0.7.11-linux-x86_64.tar.gz"
    sha256 "a9c8fe2f9639608855a7fa09f12888ce5010a3a643011490ae75cdb674312f9a"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
