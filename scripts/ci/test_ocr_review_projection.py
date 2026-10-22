#!/usr/bin/env python3
"""Offline QA contract tests; only synthetic words and bounding boxes."""
import unittest
from ocr_review_projection import Word, OcrLine, classify, metrics, parse_tsv


def line(y: int, words: list[str], *, width: int = 1300, height: int = 48, confidence: float = 97) -> OcrLine:
    import math
    x = 120
    per = max(20, width // max(1, len(words)))
    items = [Word(t, x+j*per, y, min(per-4,100), height, confidence, (1,1,1,int(y))) for j,t in enumerate(words)]
    return OcrLine(items, min(p.x for p in items), y,
                   max(p.x+p.w for p in items), y+height)


class OcrReviewContract(unittest.TestCase):
    def test_connected_short_final_line_preserved_with_image_junk_reviewed(self):
        board = line(270, ['0'], width=50, confidence=72)
        first = line(1570, ['Русские','строки','остаются','в','поиске'],confidence=90)
        second = line(1640, ['Другой','абзац','имеет','продолжение'],confidence=91)
        final = line(1715, ['заканчивается'],width=120,confidence=95)
        src=[board,first,second,final]
        result=classify(src,1700)
        self.assertEqual(result['status'],'partial')
        self.assertEqual(result['body'],[1,2,3])
        self.assertEqual(result['review'],[0])
        self.assertEqual(metrics(src,result)['search_words'],10)

    def test_high_confidence_figure_labels_are_not_indexed_merely_by_score(self):
        labels=line(145,['A1','B2','C3','D4'],width=250,confidence=99)
        paragraphs=[line(1510,['Текст','в','книге','очень','важен']),
                    line(1580,['Строки','из','книги','для','поиска'])]
        result=classify([labels,*paragraphs],1700)
        self.assertEqual(result['status'],'partial')
        self.assertEqual(result['review'],[0])
        self.assertEqual(result['body'],[1,2])

    def test_single_short_paragraph_cannot_silently_pass_complete(self):
        src=[line(1600,['Один','длинный','абзац','состоящий','из','слов'])]
        result=classify(src,1700)
        self.assertEqual(result['status'],'needs_review')
        self.assertEqual(result['body'],[])
        self.assertEqual(result['review'],[0])

    def test_two_body_lines_still_require_proofreading(self):
        src=[line(1500,['Первый','параграф','полностью','виден']),line(1580,['Второй','параграф','полностью','виден'])]
        result=classify(src,1700)
        self.assertEqual(result['status'],'partial')
        self.assertEqual(result['review'],[])
        self.assertEqual(metrics(src,result)['raw_words'],8)

    def test_reject_invalid_tsv_coordinates_and_fields(self):
        base='level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n'
        for extra in ['5\t1\t1\t1\t1\t1\t160\t20\t100\t30\t80\tTEST\n',
                      '5\t1\t1\t1\t1\t1\t10\t20\t10\t30\tnan\tTEST\n',
                      '5\t1\t1\t1\t1\t1\t10\t20\t10\t30\t101\tTEST\n']:
            with self.assertRaises(ValueError):parse_tsv(base+extra,200,100)
        with self.assertRaises(ValueError):parse_tsv('unrecognized\tcol\n',200,100)

    def test_every_line_must_be_in_exactly_one_channel(self):
        src=[line(40,['A1']),line(1600,['Это','предложение','достаточно','длинное']),
             line(1680,['Текст','точно','можно','найти'])]
        result=classify(src,1700)
        fake=dict(result,body=[1],review=[0])
        with self.assertRaises(AssertionError):metrics(src,fake)
        fake2=dict(result,status='complete')
        with self.assertRaises(AssertionError):metrics(src,fake2)

    def test_empty_page_is_needs_review_not_false_complete(self):
        result=classify([],1700)
        self.assertEqual(result['status'],'needs_review')
        self.assertEqual(metrics([],result)['raw_words'],0)

if __name__=='__main__':unittest.main()