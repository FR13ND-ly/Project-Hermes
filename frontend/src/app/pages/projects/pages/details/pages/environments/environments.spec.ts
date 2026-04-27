import { ComponentFixture, TestBed } from '@angular/core/testing';

import { Environments } from './environments';

describe('Environments', () => {
  let component: Environments;
  let fixture: ComponentFixture<Environments>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [Environments]
    })
    .compileComponents();

    fixture = TestBed.createComponent(Environments);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });
});
