export function LoginPage() {
  return (
    <main className="login">
      <h1>Rubato</h1>
      <p>Each developer orchestrates their own agents. Rubato keeps the whole picture.</p>
      <a className="button" href="/auth/github/login">
        Log in with GitHub
      </a>
    </main>
  );
}
