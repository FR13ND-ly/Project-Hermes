import { Routes } from '@angular/router';
import { Auth } from './pages/auth/auth'; // Verifică dacă importul se face corect

export const routes: Routes = [
  // 1. Când intri direct pe localhost:4200, te trimite automat la login
  { path: '', redirectTo: 'login', pathMatch: 'full' }, 
  
  // 2. Aici definim ruta pentru pagina ta
  { path: 'login', component: Auth },

  // (Opțional) Dacă vrei să pregătești și ruta pentru pagini inexistente
  // { path: '**', component: NotFoundComponent } 
];