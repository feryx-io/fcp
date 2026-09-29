from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.balance_transaction_kind import BalanceTransactionKind
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="CreateTransactionRequest")



@_attrs_define
class CreateTransactionRequest:
    """ Credit or adjustment request.

        Attributes:
            kind (BalanceTransactionKind): Why a balance changed.
            amount (float):
            currency (None | str | Unset): Currency used when this entry creates the tenant's balance row.
                Defaults to USD.
            description (None | str | Unset):
            external_ref (None | str | Unset): Reference to the external billing system's record (invoice id,
                payment id, ...).
     """

    kind: BalanceTransactionKind
    amount: float
    currency: None | str | Unset = UNSET
    description: None | str | Unset = UNSET
    external_ref: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        kind = self.kind.value

        amount = self.amount

        currency: None | str | Unset
        if isinstance(self.currency, Unset):
            currency = UNSET
        else:
            currency = self.currency

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        external_ref: None | str | Unset
        if isinstance(self.external_ref, Unset):
            external_ref = UNSET
        else:
            external_ref = self.external_ref


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "kind": kind,
            "amount": amount,
        })
        if currency is not UNSET:
            field_dict["currency"] = currency
        if description is not UNSET:
            field_dict["description"] = description
        if external_ref is not UNSET:
            field_dict["external_ref"] = external_ref

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        kind = BalanceTransactionKind(d.pop("kind"))




        amount = d.pop("amount")

        def _parse_currency(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        currency = _parse_currency(d.pop("currency", UNSET))


        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_external_ref(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        external_ref = _parse_external_ref(d.pop("external_ref", UNSET))


        create_transaction_request = cls(
            kind=kind,
            amount=amount,
            currency=currency,
            description=description,
            external_ref=external_ref,
        )


        create_transaction_request.additional_properties = d
        return create_transaction_request

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
