/** A state as a glyph and a word; `abnormal` is the only thing that adds colour. */
export interface Mark {
  glyph: string;
  word: string;
  abnormal: boolean;
}
