class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.2.0/goldbot-v0.2.0-macos-aarch64.tar.gz"
      sha256 "c4e84ef869e81418e27d15212f8c18ba6f0549c39f04bb59f9a96326322073e7"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.2.0/goldbot-v0.2.0-macos-x86_64.tar.gz"
      sha256 "1de03ef13cb8e2d94c46d43408b23d661bcdc9ba85e49d7e729525d42159b86e"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.2.0/goldbot-v0.2.0-linux-x86_64.tar.gz"
    sha256 "eb71aa8147e144ceaf3305efb24aa8de07150c3dbc895406e773db0ccd940e24"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
