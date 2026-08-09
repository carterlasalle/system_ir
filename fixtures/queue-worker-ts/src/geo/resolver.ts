/** Street-name vocabulary resolver (department-specific names). */
const DEPARTMENT_VOCAB = new Map<string, string>([
  ["Oak St", "Oak Street (Dept. A)"],
]);

export function resolveStreetName(text: string): string {
  for (const [from, to] of DEPARTMENT_VOCAB) {
    if (text.includes(from)) {
      return text.replace(from, to);
    }
  }
  return text;
}
