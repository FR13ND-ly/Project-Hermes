import { ComponentFixture, TestBed } from '@angular/core/testing';

import { Storages } from './storages';

describe('Storages', () => {
  let component: Storages;
  let fixture: ComponentFixture<Storages>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [Storages]
    })
    .compileComponents();

    fixture = TestBed.createComponent(Storages);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });
});
