#!/usr/bin/env python3
"""QA-only synthetic diagram plus authored Russian text; no user images or model training."""
from __future__ import annotations
import argparse, hashlib, json, os, random, subprocess, tempfile, time
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont
from ocr_review_projection import parse_tsv, classify, metrics
from ready_ocr_lab import quality

PAGE_SIZE=(1700,2300)
BODY=[
"Простой текст для проверки извлечения из книги.",
"Связные строки должны сохраняться без потерь.",
"На странице также находится схема для анализа.",
"Не следует превращать элементы схемы в слова.",
"Поиск должен указывать на исходную страницу.",
"Сомнительные фрагменты требуют проверки.",
]

def make_page(path:Path)->None:
    page=Image.new('RGB',PAGE_SIZE,'white')
    draw=ImageDraw.Draw(page)
    x0,y0,step=340,190,56
    for k in range(19):
        xx=x0+k*step
        draw.line((xx,y0,xx,y0+18*step),fill=(15,15,15),width=3)
        yy=y0+k*step
        draw.line((x0,yy,x0+18*step,yy),fill=(15,15,15),width=3)
    rng=random.Random(7316)
    for i in range(19):
        for j in range(19):
            if rng.random()<0.61:
                x,y=x0+i*step,y0+j*step
                r=24
                dark=rng.random()<0.49
                fill='black' if dark else 'white'
                draw.ellipse((x-r,y-r,x+r,y+r),outline='black',width=3,fill=fill)
    font=ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf',42)
    # Figure-coordinate labels are authentic diagram annotations, not prose.
    draw.text((x0+40, 75), "A1 B2 C3 D4", font=font, fill="black")
    for k,line in enumerate(BODY):
        draw.text((120,1480+81*k),line,font=font,fill='black')
    page.save(path)

def run_ocr(image:Path, out:Path):
    env=os.environ.copy();env['OMP_THREAD_LIMIT']='1'
    p=subprocess.run(['tesseract',str(image),str(out),'-l','rus+eng','--oem','1','--psm','3','tsv'],env=env,capture_output=True,timeout=60)
    if p.returncode:raise RuntimeError('Tesseract error')

def investigate(image:Path, tsv:Path)->dict:
    lines=parse_tsv(tsv.read_text('utf-8'),*PAGE_SIZE)
    outcome=classify(lines,PAGE_SIZE[0]);data=metrics(lines,outcome)
    # Independent source-space geometry oracle: all printed test body begins below y=1460.
    # This isn't used by the classifier; only for the QA assertion.
    body_y=1460
    true_words=sum(len(x.words) for x in lines if x.y>=body_y)
    returned_true_words=sum(len(lines[i].words) for i in outcome['body'] if lines[i].y>=body_y)
    indexed_figure_words=sum(len(lines[i].words) for i in outcome['body'] if lines[i].y<body_y)
    def joined(indices):
        return " ".join(" ".join(w.text for w in lines[i].words) for i in indices)
    correct = " ".join(BODY)
    raw_metric = quality(correct, joined(range(len(lines))))
    projected_metric = quality(correct, joined(outcome['body']))
    data.update({'raw_cer': raw_metric['cer'], 'raw_wer': raw_metric['wer'],
                 'projected_cer': projected_metric['cer'], 'projected_wer': projected_metric['wer'],
                 'actual_paragraph_words':true_words,
                 'kept_paragraph_words':returned_true_words,
                 'indexed_figure_words':indexed_figure_words,
                 'contains_unverified_regions':len(outcome['review'])>0})
    return data

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--djvu-rs',action='store_true')
    args=parser.parse_args()
    args.out.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='shiva-qa-go-') as tmp:
        tmp=Path(tmp);png=tmp/'synthetic.png';tsv=tmp/'synthetic'
        make_page(png)
        if args.djvu_rs:
            import djvu_rs
            pbm=tmp/'synthetic.pbm';source=tmp/'synthetic.djvu'
            Image.open(png).convert('1').save(pbm)
            proc=subprocess.run(['cjb2','-dpi','300',str(pbm),str(source)],timeout=60,capture_output=True)
            if proc.returncode:raise RuntimeError('could not encode synthetic DjVu')
            opened=djvu_rs.Document.open(str(source))
            if opened.page_count()!=1:raise AssertionError('synthetic DjVu did not contain one page')
            decoded=opened.page(0).render(dpi=300).to_pil()
            if decoded.size!=PAGE_SIZE:raise AssertionError('DjVu decode geometry differs')
            png=tmp/'rust_decoded.png';decoded.save(png)
        run_ocr(png,tsv)
        data=investigate(png,tsv.with_suffix('.tsv'))
    if data['actual_paragraph_words'] <= 15:raise AssertionError('synthetic text was not recognized')
    if data['kept_paragraph_words'] != data['actual_paragraph_words']:
        raise AssertionError('actual printed words excluded from search')
    if data['indexed_figure_words']:
        raise AssertionError('diagram OCR garbage leaked into search')
    if data['projected_cer'] > data['raw_cer']:
        raise AssertionError('projection worsened authored-source CER')
    if data['review_words']:
        if data['status']!='partial':raise AssertionError('missing Partial state')
    (args.out/'figure_quality.json').write_text(json.dumps(data,indent=2,sort_keys=True)+'\n',encoding='utf-8')
    print('SHIVA_OCR_FIGURE_REVIEW_QA_PASS',json.dumps(data,sort_keys=True))

if __name__=='__main__':main()