import React from 'react';
import { inlineCode } from '../sn.jsx';

// Mini markdown: paragraphs, "- " bullets, `inline code`.
export default function Rich({ text }) {
  const blocks = String(text).split(/\n\n+/);
  return (
    <>
      {blocks.map((b, i) => {
        const lines = b.split('\n').filter((l) => l.trim().length > 0);
        if (lines.length > 0 && lines.every((l) => l.trim().startsWith('- '))) {
          return (
            <ul key={i}>
              {lines.map((l, j) => (
                <li key={j} dangerouslySetInnerHTML={{ __html: inlineCode(l.trim().slice(2)) }} />
              ))}
            </ul>
          );
        }
        return (
          <p key={i}>
            {lines.map((l, j) => (
              <React.Fragment key={j}>
                {j > 0 && <br />}
                <span dangerouslySetInnerHTML={{ __html: inlineCode(l) }} />
              </React.Fragment>
            ))}
          </p>
        );
      })}
    </>
  );
}
