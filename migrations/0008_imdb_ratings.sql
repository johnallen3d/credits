-- IMDb's daily ratings dataset (title.ratings.tsv.gz), replaced whole on each
-- download, so episodes OMDb has no score for can still show one. Titles are
-- IMDb identifiers without their `tt` prefix. When it was last downloaded is
-- kept in provider_cache under provider `imdb`.
CREATE TABLE imdb_ratings (
    title INTEGER NOT NULL PRIMARY KEY,
    score REAL    NOT NULL,
    votes INTEGER NOT NULL
) STRICT;
