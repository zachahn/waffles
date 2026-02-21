# waffles

run tasks in parallel

## example

```sh
/usr/bin/env waffles

echo 1
echo 2
echo 3
echo 4
echo 5
echo 6 && exit 1
```

output:

```
echo 4           | 4
echo 1           | 1
echo 2           | 2
echo 5           | 5
echo 3           | 3
echo 6 && exit 1 | 6

failed:
  echo 6 && exit 1
```
