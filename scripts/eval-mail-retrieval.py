#!/usr/bin/env python3
"""Live, fiction-only local embedding relevance/null-case check. No mailbox IO.
Run after changing local models, task profiles or semantic thresholds.
"""
import json
import math
import urllib.request

QUERIES = [
    'where is my hotel receipt', 'XRF meeting notes',
    'what did they quote for licensing my music', 'invoice from the electrician',
    'zebra spacecraft opera quantum pizza',
]
DOCUMENTS = [
    'Your accommodation reservation at Lakeview Hotel is confirmed. The attached payment receipt is for your overnight stay.',
    'XRF weekly meeting minutes with Troy: the team agreed on the launch plan and next steps.',
    'Here are our rates for the beat license. An exclusive license costs $500 and a non-exclusive license costs $40.',
    'Electrical repair invoice: $250 for fixing the kitchen circuit.',
    'Our November newsletter: save twenty percent on coats and shoes.',
]
THRESHOLD = 0.65  # EmbeddingClient::minimum_similarity, embeddinggemma-2 only

def cosine(a, b):
    return sum(x*y for x,y in zip(a,b))/(math.sqrt(sum(x*x for x in a))*math.sqrt(sum(y*y for y in b)))

if __name__ == '__main__':
    texts = ['task: search result | query: '+q for q in QUERIES] + ['title: Email | text: '+d for d in DOCUMENTS]
    request = urllib.request.Request('http://127.0.0.1:11436/api/embed',
        data=json.dumps({'model':'embeddinggemma-2','input':texts}).encode(),
        headers={'Content-Type':'application/json'})
    vectors = json.load(urllib.request.urlopen(request,timeout=35))['embeddings']
    assert len(vectors) == len(texts)
    scores = [[cosine(q,d) for d in vectors[len(QUERIES):]] for q in vectors[:len(QUERIES)]]
    for i, row in enumerate(scores):
        found = [j for j,s in enumerate(row) if s >= THRESHOLD]
        expected = [i] if i < 4 else []
        assert found == expected, 'Query {}: expected {}, got {}; scores {}'.format(i,expected,found,row)
    print('PASS: 4 paraphrase matches, 21 unrelated pairs rejected; no mailbox data used.')
