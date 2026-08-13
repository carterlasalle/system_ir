import { useEffect } from "react";

export function handleTheme(): void {}

export function handleClick(): void {}

export const App = () => {
  useEffect(handleTheme, []);
  document.addEventListener("click", handleClick);
  const apiUrl = process.env.API_URL;
  return <div>{apiUrl}</div>;
};

export default App;
