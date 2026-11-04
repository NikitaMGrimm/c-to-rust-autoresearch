// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>

void printIntPtrLine(const int *intNumber)
{
    printf("%d\n", *intNumber);
}

void bad()
{
    int *data;
    printIntPtrLine(data);
}

void good()
{
    int data;
    data = 5;
    int *data_addr;
    data_addr = &data;
    printIntPtrLine(data_addr);
}

int main()
{
    int x = 0;
    scanf("%d", &x);

    if (x)
    {
        good();
    }
    else
    {
        bad();
    }
    return 0;
}
