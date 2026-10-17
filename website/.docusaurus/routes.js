import React from 'react';
import ComponentCreator from '@docusaurus/ComponentCreator';

export default [
  {
    path: '/gpui-visual-test/',
    component: ComponentCreator('/gpui-visual-test/', '7e2'),
    exact: true
  },
  {
    path: '*',
    component: ComponentCreator('*'),
  },
];
