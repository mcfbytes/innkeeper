# Attribution

`opentdb.json` is a snapshot of the questions in the Open Trivia Database, a free, user-contributed trivia question database.

The questions and category names were retrieved from the Open Trivia Database API at <https://opentdb.com>. The database is published under the Creative Commons Attribution-ShareAlike 4.0 International License (CC BY-SA 4.0):

<https://creativecommons.org/licenses/by-sa/4.0/>

The legal code of that license is in `LICENSE`.

The snapshot was fetched with `fetch_opentdb.py` on 2026-10-18. Question text, answers, category, type, and difficulty are unmodified apart from decoding the API's `url3986` (RFC 3986 percent-encoding) into Unicode. The file adds a stable id for each question (SHA-1 of the category and question text), drops a repeated category+question row when the API returns one, and sorts the questions by that id.
