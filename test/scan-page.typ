// The page the fake scanner of test/printer-debug.sh "scans": an article page deriving the wave equation.
#set page(paper: "a4", margin: (x: 1.8cm, y: 2cm), columns: 2, fill: rgb("#f5f1e6"),
  header: text(8pt)[Revista de Física Didáctica · Vol. 12, n.º 3 #h(1fr) 117],
  footer: align(center, text(8pt)[— 117 —]))
#set text(lang: "es", size: 9.5pt)
#set par(justify: true)
#set heading(numbering: "1.")
#set math.equation(numbering: "(1)")
#show heading: set text(10.5pt)

#place(top, scope: "parent", float: true, align(center)[
  #text(16pt, weight: "bold")[La cuerda vibrante: deducción de la ecuación de onda]
  #v(2pt)
  A. Pérez y L. Martín · _Departamento de Física_
  #v(4pt)
  #block(width: 88%, text(8.5pt, align(left)[*Resumen.* Partiendo de la segunda ley de Newton aplicada a un
  elemento de una cuerda tensa, se obtiene la ecuación de onda unidimensional, se presenta la solución de
  d'Alembert y se calculan los modos normales de una cuerda de extremos fijos.]))
])

= Planteamiento

Sea una cuerda de densidad lineal $mu$ sometida a una tensión $T$ y estirada a lo largo del eje $x$.
Llamamos $y(x, t)$ a su desplazamiento vertical y suponemos oscilaciones pequeñas, de modo que la
pendiente cumple $|partial y \/ partial x| << 1$ y la tensión es la misma en toda la cuerda.

= Fuerzas sobre un elemento

Sobre el trozo comprendido entre $x$ y $x + d x$ actúan las tensiones de sus extremos, que forman
ángulos $theta(x)$ y $theta(x + d x)$ con la horizontal. La fuerza vertical neta es
$ F_y = T sin theta(x + d x) - T sin theta(x). $
Para ángulos pequeños, $sin theta approx tan theta = partial y \/ partial x$, así que
$ F_y approx T [ (partial y)/(partial x) ]_x^(x + d x) approx T (partial^2 y)/(partial x^2) d x. $

= La ecuación de onda

La masa del elemento es $mu d x$ y su aceleración vertical, $partial^2 y \/ partial t^2$. Por la
segunda ley de Newton,
$ mu d x (partial^2 y)/(partial t^2) = T (partial^2 y)/(partial x^2) d x, $
y al dividir por $mu d x$ queda
$ (partial^2 y)/(partial t^2) = c^2 (partial^2 y)/(partial x^2), quad c = sqrt(T/mu). $ <onda>
La constante $c$ tiene dimensiones de velocidad: es la rapidez con que se propagan las
perturbaciones por la cuerda.

= Solución de d'Alembert

Con el cambio de variables $xi = x - c t$, $eta = x + c t$, la @onda se reduce a
$partial^2 y \/ partial xi partial eta = 0$, cuya solución general es
$ y(x, t) = f(x - c t) + g(x + c t), $
la suma de dos ondas que viajan sin deformarse en sentidos opuestos. Si inicialmente
$y(x, 0) = phi(x)$ y $partial_t y(x, 0) = psi(x)$, se obtiene
$ y = 1/2 [phi(x - c t) + phi(x + c t)] + 1/(2c) integral_(x - c t)^(x + c t) psi d s. $

= Modos normales

Si la cuerda tiene longitud $L$ y sus extremos están fijos, $y(0, t) = y(L, t) = 0$. Separando
variables, $y = X(x) thin T(t)$, se obtienen las soluciones estacionarias
$ y_n (x, t) = sin((n pi x)/L) cos(omega_n t), quad omega_n = (n pi c)/L, $
con $n = 1, 2, 3, dots$ La frecuencia fundamental es $f_1 = c \/ 2L = sqrt(T \/ mu) \/ 2L$: tensar
la cuerda la hace sonar más aguda, y acortarla también.

#figure(
  {
    let (w, h) = (6.6cm, 1.1cm)
    let mode(n) = curve(stroke: 0.7pt, curve.move((0pt, h)),
      ..range(1, 81).map(i => curve.line((w * i / 80, h - h * calc.sin(n * calc.pi * i / 80)))))
    stack(spacing: 6pt, ..(1, 2, 3).map(n => box(width: w, height: 2 * h,
      { place(line(start: (0pt, h), end: (w, h), stroke: 0.3pt + gray)); place(mode(n)) })))
  },
  caption: [Los tres primeros modos normales, $n = 1, 2, 3$.],
)

= Conclusiones

La ecuación de onda surge de aplicar la segunda ley de Newton a cada elemento de la cuerda en el
límite de pequeñas oscilaciones. Su solución general son dos ondas viajeras, y las condiciones de
contorno seleccionan un conjunto discreto de frecuencias, origen de los armónicos de los
instrumentos de cuerda.
