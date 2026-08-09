/** Client component: renders a transcript record. */
export function TranscriptView({ record }: { record: { transcript: string } }) {
  return <p>{record.transcript}</p>;
}
