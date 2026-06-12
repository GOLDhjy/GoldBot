class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.9.22"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.22/goldbot-v0.9.22-macos-aarch64.tar.gz"
      sha256 "a03204d7f102b3d065391ae358a129fed5a7bed68b63ba27059a948e878b7307"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.22/goldbot-v0.9.22-macos-x86_64.tar.gz"
      sha256 "a599a5b6511f095686b8358bafe4caac82d9165ab5454bb378623a7ef8b25dd1"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.9.22/goldbot-v0.9.22-linux-x86_64.tar.gz"
    sha256 "bb5cfc99d9c0aa961e24d2d9273359d1b2e160b4ba905456a97521002bb44db0"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
