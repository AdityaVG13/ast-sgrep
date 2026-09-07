"""fixture for pass-56 nondeterminism repro.

mirrors the pass-54 R3 literal-lane cell family: calc(...) call sites
with whitespace/comment/trailing-comma variants (sg isomorphic-tree rule).
"""
def calc(a, b):
    return a + b

def work():
    x = calc(1, 2)
    y = calc( 1,  2 )
    z = calc(1, 2,)  # pattern-side comma is significant; this one has one
    w = calc(/* lead */ 1, 2)
    return x + y

def more():
    q = calc( 1,2 )
    return q
