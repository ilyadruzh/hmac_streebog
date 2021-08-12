# ZoKrates


## Установка инструментов ZoKrates

```bash
curl -LSfs get.zokrat.es | sh
```

## Инструкции 

```bash
zokrates compile -i streebog_step_1.zok -o streebog_constr_1 --ztf

zokrates compute-witness -i streebog_constr_1 -a 0 0 0 0 0 0 3 2 0 0 0 0 0 0 5 5

zokrates compute-witness --verbose -a 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15
```

Полученное свидетельство записываем в новый файл streebog_step_2.zok

```bash
zokrates compile -i stdlib/hmac/streebog2.zok -o streebog_constr_2 --ztf
zokrates setup
zokrates export-verifier
zokrates compute-witness -i streebog_constr_2 -a 0 0 0 0 0 0 3 2 0 0 0 0 0 0 5 5
zokrates generate-proof -i streebog_constr_2 

```
