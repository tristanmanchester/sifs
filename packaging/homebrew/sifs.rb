class Sifs < Formula
  desc "SIFS Is Fast Search: instant local code search for agents"
  homepage "https://github.com/tristanmanchester/sifs"
  url "https://crates.io/api/v1/crates/sifs/0.4.0/download"
  sha256 "2022785cec52c019d4fd8590cd3cb94e42f2e3b6631a1355fbd1e9c71d4a9af6"
  version "0.4.0"
  license "MIT"
  head "https://github.com/tristanmanchester/sifs.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", "--path", ".", "--root", prefix
  end

  test do
    repo = testpath/"tiny-repo"
    repo.mkpath
    (repo/"auth.py").write <<~PY
      def authenticate_token(token):
          return token == "secret"
    PY

    system "git", "-C", repo, "init", "--quiet"

    output = shell_output("#{bin}/sifs search authenticate_token --source #{repo} --mode bm25 --offline --no-cache")
    assert_match "auth.py", output
    assert_match "authenticate_token", output
  end
end
