"""An original, low-key electronic score. No samples or third-party music."""
import numpy as np
import wave
from pathlib import Path
rate=48000
duration=140
audio=np.zeros((rate*duration,2),np.float32)
rng=np.random.default_rng(27926)
def add(start,signal,pan=0.0,gain=1.0):
    offset=int(start*rate)
    if offset<0:return
    count=min(len(signal),len(audio)-offset)
    if count<=0:return
    audio[offset:offset+count,0]+=signal[:count]*gain*np.sqrt((1-pan)/2)
    audio[offset:offset+count,1]+=signal[:count]*gain*np.sqrt((1+pan)/2)
def hz(midi):return 440*2**((midi-69)/12)
# Four warm suspended/minor voicings, eight seconds each.
chords=[[45,52,59,64],[41,48,55,60],[48,55,62,67],[43,50,57,62]]
for bar,start in enumerate(np.arange(0,duration,8)):
    notes=chords[bar%4]
    t=np.arange(int(9*rate))/rate
    env=(1-np.exp(-t/1.2))*np.exp(-np.maximum(0,t-6)/1.4)
    for j,note in enumerate(notes):
        f=hz(note)
        wavelet=(np.sin(2*np.pi*f*t)+.38*np.sin(2*np.pi*(f*1.0015)*t+.4)+.11*np.sin(2*np.pi*2*f*t))*.025*env
        add(start,wavelet,pan=(j-1.5)/2.5)
    # Rounded bass, with enough space for on-screen reading.
    for step in range(8):
        at=start+step
        bt=np.arange(int(.8*rate))/rate
        bass=np.sin(2*np.pi*hz(notes[0]-12)*bt)*(1-np.exp(-bt*75))*np.exp(-bt*5)*.085
        add(at,bass,0)
    for step in range(16):
        at=start+step*.5
        nt=np.arange(int(1.8*rate))/rate
        f=hz(notes[[1,2,3,2,1,3,2,3][step%8]]+12)
        note=(np.sin(2*np.pi*f*nt)+.18*np.sin(2*np.pi*f*2*nt))*np.exp(-nt*4)*(1-np.exp(-nt*120))*.032
        add(at,note,pan=(-.65 if step%2 else .65))
        add(at+.375,note*.25,pan=(.65 if step%2 else -.65))
        if step%2==0 and start>=8:
            kt=np.arange(int(.3*rate))/rate
            freq=48+70*np.exp(-kt*28)
            kick=np.sin(2*np.pi*np.cumsum(freq)/rate)*np.exp(-kt*17)*.063
            add(at,kick)
    if start>=14:
        for step in range(16):
            ht=np.arange(int(.06*rate))/rate
            noise=rng.normal(0,1,len(ht));noise=np.diff(noise,prepend=0)
            add(start+step*.5,noise*np.exp(-ht*85)*.006,pan=.25)
# Small airy swells at editorial chapter boundaries.
for at in [8,14,26,36,48,60,66,78,88,98,108,120,134]:
    t=np.arange(rate)/rate
    noise=rng.normal(0,1,len(t))
    noise=np.convolve(noise,np.ones(28)/28,mode='same')
    add(at-.65,noise*np.sin(np.pi*t)**3*.06,pan=-.2)
fade=np.minimum(np.arange(len(audio))/rate/2,1)*np.minimum((len(audio)-np.arange(len(audio)))/rate/3,1)
audio*=fade[:,None]
audio=np.tanh(audio*1.5)*.8*2.5118864315
root=Path(__file__).resolve().parents[1]
with wave.open(str(root/'assets/soundtrack.wav'),'wb') as f:
    f.setnchannels(2);f.setsampwidth(2);f.setframerate(rate);f.writeframes((audio*32767).astype('<i2').tobytes())
print('Original stereo score: 140 seconds at 48 kHz')
