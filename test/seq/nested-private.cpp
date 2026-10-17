#include <cstdio>
//%%
class List {
  struct Node;
  Node *head = nullptr;
public:
  void push(int v);
  int top() const;
};
//%%
struct List::Node {
  int v;
  Node *next;
  Node(int v, Node *next);
};
//%%
List::Node::Node(int v, Node *next) : v(v), next(next) {}
//%%
void List::push(int v) { head = new Node(v, head); }
//%%
int List::top() const { return head->v; }
//%%
List::Node outside(1, nullptr);
//%%
int main() { List l; l.push(1); l.push(2); std::printf("%d\n", l.top()); }
