import { request } from "./api";

export function App() {
  request("/api/me");
  return <div>hi</div>;
}
