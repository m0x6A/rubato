import { keepRouteForLogin } from "../../shared/route.ts";

/** The whole page while nobody is logged in. The view the link asked for is kept across the login (issue #357). */
export function LoginPage() {
  return (
    <main className="login">
      <h1>Rubato</h1>
      <p>Each developer orchestrates their own agents. Rubato keeps the whole picture.</p>
      <a className="button" href="/auth/github/login" onClick={keepRouteForLogin}>
        Log in with GitHub
      </a>
    </main>
  );
}
