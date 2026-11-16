#!/usr/bin/env python3
"""Live, fiction-only local embedding relevance/null-case check. No mailbox IO.
Run after changing local models, task profiles or semantic thresholds.
"""
import json
import re
from pathlib import Path
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
SOURCE = Path(__file__).resolve().parents[1] / 'src-tauri/crates/cxmail-email/src/email/embeddings.rs'
THRESHOLD = float(re.search(r'const LOCAL_VAULT_MIN_SIMILARITY: f32 = ([0-9.]+);', SOURCE.read_text()).group(1))
FILLER = 'Hello Chris, thanks for getting in touch. We appreciate your time and look forward to working with you. Please review the details below, keep this email for your records, and let us know if you have any questions. '
LONG_QUERIES = ['the hotel payment receipt from my trip', 'what license price did they quote for my music', 'XRF meeting decisions and notes']
LONG_FACTS = [
    'Your payment receipt for the overnight accommodation at Lakeview Hotel is attached. Paid $250 for your stay.',
    'Our next travel newsletter includes hotel offers and tips for planning your trip. No booking or payment has been made.',
    'Thanks for asking about the beat. I can offer a non-exclusive license for $40 or an exclusive license for $500.',
    'This month our music production newsletter covers mixing and arranging your tracks. Our free workshop is on Friday.',
    'XRF meeting minutes with Troy. We decided to approve the launch plan and assign the report to Alex. Here are the next steps.',
    'Riverbend team meeting minutes. We discussed the launch and assigned the report to Alex. Here are the next steps.',
]

def embed(texts):
    vectors=[]
    for i in range(0,len(texts),2):
        request = urllib.request.Request('http://127.0.0.1:11436/api/embed',
            data=json.dumps({'model':'embeddinggemma-2','input':texts[i:i+2]}).encode(),
            headers={'Content-Type':'application/json'})
        vectors += json.load(urllib.request.urlopen(request,timeout=35))['embeddings']
    assert len(vectors)==len(texts)
    return vectors

def cosine(a, b):
    return sum(x*y for x,y in zip(a,b))/(math.sqrt(sum(x*x for x in a))*math.sqrt(sum(y*y for y in b)))

if __name__ == '__main__':
    texts = ['task: search result | query: '+q for q in QUERIES] + ['title: Email | text: '+d for d in DOCUMENTS]
    vectors = embed(texts)
    scores = [[cosine(q,d) for d in vectors[len(QUERIES):]] for q in vectors[:len(QUERIES)]]
    for i, row in enumerate(scores):
        found = [j for j,s in enumerate(row) if s >= THRESHOLD]
        expected = [i] if i < 4 else []
        assert found == expected, 'Query {}: expected {}, got {}; scores {}'.format(i,expected,found,row)
    long_texts = ['task: search result | query: '+q for q in LONG_QUERIES] + ['title: Email | text: '+(FILLER*3+d+' '+FILLER*3)[:1600] for d in LONG_FACTS]
    long_vectors=embed(long_texts)
    for i,q in enumerate(long_vectors[:len(LONG_QUERIES)]):
        row=[cosine(q,d) for d in long_vectors[len(LONG_QUERIES):]]
        found=[j for j,s in enumerate(row) if s >= THRESHOLD]
        assert found==[i*2], 'Long query {}: expected {}, got {}; scores {}'.format(i,[i*2],found,row)
    print('PASS: 7 paraphrase matches, 36 unrelated pairs rejected (including 1,600-character mail and similar-topic distractors); no mailbox data used.')
