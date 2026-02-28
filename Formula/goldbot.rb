class Goldbot < Formula
  desc "GoldBot TUI Automation Agent"
  homepage "https://github.com/GOLDhjy/GoldBot"
  version "0.8.12"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.12/goldbot-v0.8.12-macos-aarch64.tar.gz"
      sha256 "c7579e4cb55a984f4923aaeacdaa61cd46411f723d9a78e196198d31d7ff6a4b"
    else
      url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.12/goldbot-v0.8.12-macos-x86_64.tar.gz"
      sha256 "05af35b638e9755876abc690d58cef6320ae35ab555a5064ced603df4b6f9646"
    end
  end

  on_linux do
    url "https://github.com/GOLDhjy/GoldBot/releases/download/v0.8.12/goldbot-v0.8.12-linux-x86_64.tar.gz"
    sha256 "3e93f6a05aa79083817e17644e61f7efbcf33c95e2522b28568757c54e792ab3"
  end

  def install
    bin.install "goldbot"
  end

  test do
    assert_match "GoldBot", shell_output("#{bin}/goldbot --help")
  end
end
