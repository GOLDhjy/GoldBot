class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.11"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.11/goldbot-v0.9.11-macos-aarch64.tar.gz"
      sha256 "2c803af5afb0125bc8da7fee43b7a0c7672569aa3575601c1cd157e3920beea9"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.11/goldbot-v0.9.11-macos-x86_64.tar.gz"
      sha256 "03f722184d252a1870a227e54ec0262ededcffe8abc52bb6d0d8f0086b831177"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.11/goldbot-v0.9.11-linux-x86_64.tar.gz"
    sha256 "f46824079c6cb37782b1742db2c57fccc224e53ae529a2f80f612ab33e120658"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
